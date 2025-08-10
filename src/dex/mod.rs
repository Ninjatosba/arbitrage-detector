//! DEX (Uniswap V3) interaction layer.
//!
//! Streams on-chain price data using a Uniswap V3 pool’s `slot0` value.
//! Converts `sqrtPriceX96` to a human-readable USDC/ETH price with high
//! precision via `bigdecimal`.

use std::{str::FromStr, sync::Arc, time::Duration};

use crate::dex::state::approx_sqrt_price_x96_at_tick;
use anyhow::Result;
use bigdecimal::{BigDecimal, Zero};
use ethers::{
    contract::abigen,
    providers::{Http, Provider},
    types::{Address, U256},
};
use num_bigint::BigInt;
use num_traits::One;
use tokio::time::sleep;

pub mod calc;
pub mod state;

pub use calc::{
    SolveToPriceResult, SwapDirection, solve_for_target_avg_price_multi_tick,
    solve_token0_in_for_target_avg_price,
};
pub use calc::{solve_from_cex_bid, solve_from_cex_top};
pub use state::{PoolState, PriceSegment, TradeCosts};

abigen!(
    UniswapV3Pool,
    r#"[
        function slot0() view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)
        function liquidity() view returns (uint128)
        function fee() view returns (uint24)
        function tickSpacing() view returns (int24)
        function ticks(int24) view returns (uint128 liquidityGross, int128 liquidityNet, uint256 feeGrowthOutside0X128, uint256 feeGrowthOutside1X128, int56 tickCumulativeOutside, uint160 secondsPerLiquidityOutsideX128, uint32 secondsOutside, bool initialized)
        function tickBitmap(int16) view returns (uint256)
    ]"#,
);

/// Handle for interacting with a specific Uniswap V3 pool.
#[derive(Clone)]
pub struct Dex {
    pool: UniswapV3Pool<Provider<Http>>,
}

impl Dex {
    pub async fn new(rpc_url: &str, pool_addr: Address) -> Result<Self> {
        let provider = Arc::new(Provider::<Http>::try_from(rpc_url)?);
        let pool = UniswapV3Pool::new(pool_addr, provider);
        pool.slot_0().call().await?; // sanity-check
        Ok(Self { pool })
    }

    /// Build a `PoolState` snapshot for pricing. For multi-tick traversal, caller can
    /// optionally pass the current tick's lower/upper sqrt price to bound the first leg.
    pub async fn get_pool_state(
        &self,
        token0_decimals: u8,
        token1_decimals: u8,
        current_tick_lower_sqrt_q96: Option<U256>,
        current_tick_upper_sqrt_q96: Option<U256>,
        max_segments_per_side: usize,
    ) -> Result<crate::dex::state::PoolState> {
        let (sqrt_price_x96, tick, _, _, _, _fee_protocol, _unlocked) =
            self.pool.slot_0().call().await?;
        let liquidity = self.pool.liquidity().call().await?;
        let tick_spacing = self.pool.tick_spacing().call().await?;

        // Fill lower/upper sqrt bounds if not provided
        let (lower_q96, upper_q96) =
            match (current_tick_lower_sqrt_q96, current_tick_upper_sqrt_q96) {
                (Some(l), Some(u)) => (Some(l), Some(u)),
                _ => {
                    let ts = tick_spacing as i32;
                    let base = tick - (tick % ts);
                    let lower_tick = base;
                    let upper_tick = base + ts;
                    (
                        Some(crate::dex::state::approx_sqrt_price_x96_at_tick(lower_tick)),
                        Some(crate::dex::state::approx_sqrt_price_x96_at_tick(upper_tick)),
                    )
                }
            };

        // Build piecewise segments beyond the current tick (bounded depth)
        let (segments_down, segments_up) = self
            .build_segments(
                tick as i32,
                tick_spacing as i32,
                liquidity,
                max_segments_per_side,
            )
            .await?;
        Ok(crate::dex::state::PoolState::new(
            sqrt_price_x96,
            liquidity,
            tick as i32,
            token0_decimals,
            token1_decimals,
            lower_q96,
            upper_q96,
            segments_down,
            segments_up,
        ))
    }

    /// Reads the Uniswap V3 pool fee (in basis points, e.g., 500 = 0.05%).
    pub async fn get_pool_fee_bps(&self) -> Result<u32> {
        let fee_raw: u32 = self.pool.fee().call().await?;
        Ok(fee_raw)
    }

    /// Spawn a background task that periodically refreshes `PoolState` and sends it via `watch`.
    pub async fn spawn_pool_state_watcher(
        &self,
        token0_decimals: u8,
        token1_decimals: u8,
        interval_secs: u64,
        max_segments_per_side: usize,
        tx: tokio::sync::watch::Sender<crate::dex::state::PoolState>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let this = self.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                if let Ok(state) = this
                    .get_pool_state(
                        token0_decimals,
                        token1_decimals,
                        None,
                        None,
                        max_segments_per_side,
                    )
                    .await
                {
                    let _ = tx.send(state);
                }
            }
        });
        Ok(handle)
    }

    async fn build_segments(
        &self,
        start_tick: i32,
        tick_spacing: i32,
        start_liquidity: u128,
        max_segments_per_side: usize,
    ) -> Result<(Vec<PriceSegment>, Vec<PriceSegment>)> {
        let mut up_segments: Vec<PriceSegment> = Vec::new();
        let mut down_segments: Vec<PriceSegment> = Vec::new();

        // Current tick boundaries for start points
        let base = start_tick - (start_tick % tick_spacing);
        let current_upper_tick = base + tick_spacing;
        let current_lower_tick = base;
        let mut curr_liq_up: i128 = start_liquidity as i128;
        let mut curr_liq_down: i128 = start_liquidity as i128;

        // Build upwards ----------------------------------------------------
        let mut from_s = approx_sqrt_price_x96_at_tick(current_upper_tick);
        let mut iter_tick = current_upper_tick;
        for _ in 0..max_segments_per_side {
            if let Some(next_tick) = self
                .next_initialized_tick_up(iter_tick, tick_spacing)
                .await?
            {
                let to_s = approx_sqrt_price_x96_at_tick(next_tick);
                if curr_liq_up > 0 {
                    up_segments.push(PriceSegment {
                        start_sqrt_price_x96: from_s,
                        end_sqrt_price_x96: to_s,
                        liquidity: curr_liq_up as u128,
                    });
                }
                // Apply liquidity net at the initialized tick (crossing up)
                let (_, liq_net, _, _, _, _, _, initialized) =
                    self.pool.ticks(next_tick).call().await?;
                if initialized {
                    curr_liq_up = curr_liq_up.saturating_add(liq_net as i128);
                }
                iter_tick = next_tick;
                from_s = to_s;
            } else {
                break;
            }
        }

        // Build downwards --------------------------------------------------
        let mut from_s_down = approx_sqrt_price_x96_at_tick(current_lower_tick);
        let mut iter_tick_down = current_lower_tick;
        for _ in 0..max_segments_per_side {
            if let Some(prev_tick) = self
                .prev_initialized_tick_down(iter_tick_down, tick_spacing)
                .await?
            {
                let to_s = approx_sqrt_price_x96_at_tick(prev_tick);
                if curr_liq_down > 0 {
                    down_segments.push(PriceSegment {
                        start_sqrt_price_x96: from_s_down,
                        end_sqrt_price_x96: to_s,
                        liquidity: curr_liq_down as u128,
                    });
                }
                // Apply liquidity net when crossing downwards (subtract)
                let (_, liq_net, _, _, _, _, _, initialized) =
                    self.pool.ticks(prev_tick).call().await?;
                if initialized {
                    curr_liq_down = curr_liq_down.saturating_sub(liq_net as i128);
                }
                iter_tick_down = prev_tick;
                from_s_down = to_s;
            } else {
                break;
            }
        }

        Ok((down_segments, up_segments))
    }

    async fn next_initialized_tick_up(
        &self,
        from_tick: i32,
        tick_spacing: i32,
    ) -> Result<Option<i32>> {
        let mut comp = (from_tick / tick_spacing) + 1; // next compressed tick
        for _ in 0..64 {
            // scan up to 64 words (16384 ticks)
            let word_pos: i16 = (comp >> 8) as i16;
            let bit_pos_start = (comp & 0xff) as usize;
            let bitmap = self.pool.tick_bitmap(word_pos).call().await?;
            // scan bits from bit_pos_start..256
            let mut found: Option<usize> = None;
            for b in bit_pos_start..256 {
                let mask = U256::one() << b;
                if (bitmap & mask) != U256::zero() {
                    found = Some(b);
                    break;
                }
            }
            if let Some(bit) = found {
                let next_comp = ((word_pos as i32) << 8) + (bit as i32);
                return Ok(Some(next_comp * tick_spacing));
            }
            // advance to next word
            comp = ((word_pos as i32) + 1) << 8;
        }
        Ok(None)
    }

    async fn prev_initialized_tick_down(
        &self,
        from_tick: i32,
        tick_spacing: i32,
    ) -> Result<Option<i32>> {
        let mut comp = (from_tick / tick_spacing) - 1; // previous compressed tick
        for _ in 0..64 {
            // scan up to 64 words (16384 ticks)
            let word_pos: i16 = (comp >> 8) as i16;
            let bit_pos_start = (comp & 0xff) as i32;
            let bitmap = self.pool.tick_bitmap(word_pos).call().await?;
            // scan bits from bit_pos_start down to 0
            let mut found: Option<i32> = None;
            for b in (0..=bit_pos_start).rev() {
                let mask = U256::one() << (b as usize);
                if (bitmap & mask) != U256::zero() {
                    found = Some(b);
                    break;
                }
            }
            if let Some(bit) = found {
                let prev_comp = ((word_pos as i32) << 8) + bit;
                return Ok(Some(prev_comp * tick_spacing));
            }
            // move to previous word
            comp = (((word_pos as i32) - 1) << 8) + 255;
        }
        Ok(None)
    }

    pub async fn stream_slot0(&self, interval_secs: u64) -> Result<()> {
        loop {
            let (sqrt_price_x96, tick, _, _, _, fee_protocol, _) =
                self.pool.slot_0().call().await?;

            let price = Self::price_usdc_per_eth(sqrt_price_x96);
            println!(
                "price={} USDC/ETH tick={} feeProtocol={}",
                price, tick, fee_protocol
            );
            sleep(Duration::from_secs(interval_secs)).await;
        }
    }

    /// Converts `sqrtPriceX96` (token1/token0) to a human-readable USDC per ETH price.
    /// Assumes token0=WETH (18 decimals) token1=USDC (6 decimals).
    pub async fn fetch_price_usdc_per_eth(&self) -> Result<BigDecimal> {
        let (sqrt_price_x96, _, _, _, _, _, _) = self.pool.slot_0().call().await?;
        Ok(Self::price_usdc_per_eth(sqrt_price_x96))
    }

    pub fn price_usdc_per_eth(sqrt_price_x96: U256) -> BigDecimal {
        // Convert sqrtPriceX96 -> BigDecimal
        let sqrt_bd = BigDecimal::from_str(&sqrt_price_x96.to_string())
            .unwrap_or_else(|_| BigDecimal::zero());
        // 2^96 as BigDecimal
        let two_pow_96 = BigDecimal::from(BigInt::one() << 96u32);

        // ratio = sqrtPriceX96 / 2^96        (high-precision division)
        let ratio = &sqrt_bd / &two_pow_96;

        // price_raw = ratio^2               (token1/token0 in raw units)
        let price_raw = &ratio * &ratio;

        // Adjust for decimal difference (10^(dec0 - dec1) = 10^12)
        let scale_adjust = BigDecimal::from(10u64.pow(12));
        let mut price = scale_adjust / price_raw;

        // Round to 4 decimals for display
        price = price.with_scale(4);
        price
    }
}
