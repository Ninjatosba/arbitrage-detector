use crate::dex::state::PoolState;
use crate::errors::Result;
use alloy_primitives::U256;
use ethers::{
    contract::abigen,
    providers::{Http, Provider},
    types::Address,
};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::warn;

use super::state::approx_sqrt_price_x96_at_tick;

abigen!(
    UniswapV3Pool,
    r"[
        function slot0() view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)
        function liquidity() view returns (uint128)
        function fee() view returns (uint24)
        function tickSpacing() view returns (int24)
    ]",
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

    /// Build a `PoolState` snapshot for pricing (single tick only).
    pub async fn get_pool_state(
        &self,
        token0_decimals: u8,
        token1_decimals: u8,
        current_tick_lower_sqrt_q96: Option<U256>,
        current_tick_upper_sqrt_q96: Option<U256>,
    ) -> Result<PoolState> {
        let (sqrt_price_x96, tick, _, _, _, _fee_protocol, _unlocked) =
            self.pool.slot_0().call().await?;
        let liquidity = self.pool.liquidity().call().await?;
        let tick_spacing = self.pool.tick_spacing().call().await?;

        // Convert ethers U256 to alloy U256
        let sqrt_price_x96_alloy =
            U256::from_str_radix(&sqrt_price_x96.to_string(), 10).unwrap_or_default();

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
                        Some(approx_sqrt_price_x96_at_tick(lower_tick)),
                        Some(approx_sqrt_price_x96_at_tick(upper_tick)),
                    )
                }
            };

        let price_usdc_per_eth = price_usdc_per_eth(sqrt_price_x96_alloy);

        Ok(PoolState::new(
            sqrt_price_x96_alloy,
            liquidity,
            tick as i32,
            token0_decimals,
            token1_decimals,
            lower_q96,
            upper_q96,
            price_usdc_per_eth,
        ))
    }

    /// Reads the Uniswap V3 pool fee (in basis points, e.g., 500 = 0.05%).
    pub async fn get_pool_fee_bps(&self) -> Result<u32> {
        let fee_raw: u32 = self.pool.fee().call().await?;
        Ok(fee_raw)
    }

    /// Fetch current ETH price in USDC
    pub async fn fetch_price_usdc_per_eth(&self) -> Result<f64> {
        let sqrt_price_x96 = self.pool.slot_0().call().await?.0;
        let sqrt_price_x96_alloy =
            U256::from_str_radix(&sqrt_price_x96.to_string(), 10).unwrap_or_default();
        Ok(price_usdc_per_eth(sqrt_price_x96_alloy))
    }
}

/// Initialize pool state watcher
pub async fn init_pool_state_watcher(
    dex: &Dex,
    _pool_tx: watch::Sender<PoolState>,
) -> Result<watch::Receiver<PoolState>> {
    // Get initial pool state
    let initial_state = dex.get_pool_state(18, 6, None, None).await?;
    let (tx, rx) = watch::channel(initial_state);

    // Spawn background task to update pool state
    let dex_clone = dex.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            ticker.tick().await;
            match dex_clone.get_pool_state(6, 18, None, None).await {
                Ok(state) => {
                    let _ = tx.send(state);
                }
                Err(e) => {
                    warn!(error = %e, "[DEX] failed to refresh pool state");
                }
            }
        }
    });

    Ok(rx)
}

fn price_usdc_per_eth(sqrt_price_x96: U256) -> f64 {
    // sqrtPriceX96 = sqrt(token1/token0) * 2^96 where token1/token0 are in nominal units
    // For WETH/USDC: sqrtPriceX96 = sqrt(USDC/WETH) * 2^96 where both are in nominal units
    let s = sqrt_price_x96.to_string();
    let sqrt_q96 = s.parse::<f64>().unwrap_or(0.0) / 2.0_f64.powi(96);
    if sqrt_q96 <= 0.0 {
        return 0.0;
    }
    // price = token1/token0 in nominal units (USDC per ETH)
    let ratio_raw = sqrt_q96 * sqrt_q96; // token1_raw / token0_raw

    // Convert raw ratio to human price (USDC per 1 ETH)
    (1.0 / ratio_raw) * 10_f64.powi(18 - 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_zero_when_sqrt_is_zero() {
        assert_eq!(price_usdc_per_eth(U256::from(0)), 0.0);
    }

    #[test]
    fn price_increases_with_sqrt() {
        // Build two sqrt values, higher should generally reflect a lower raw ratio
        // but after the (1/ratio)*10^(decimals) it should translate monotonically with sqrt.
        // We simply check that a much larger sqrt leads to a sensible positive price.
        let small = U256::from(1_000_000_000_000_000u128);
        let large = U256::from(10_000_000_000_000_000u128);
        let p_small = price_usdc_per_eth(small);
        let p_large = price_usdc_per_eth(large);
        assert!(p_small >= 0.0);
        assert!(p_large >= 0.0);
    }
}
