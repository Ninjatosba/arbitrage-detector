//! DEX integration for Uniswap V3 pools.

use crate::dex::state::approx_sqrt_price_x96_at_tick;
use anyhow::Result;
use ethers::{
    contract::abigen,
    providers::{Http, Provider},
    types::{Address, U256},
};
use std::sync::Arc;
use tokio::sync::watch;

pub mod calc;
pub mod state;

/// Spawn a background task that periodically fetches DEX price and sends it via `watch`.
pub async fn spawn_dex_price_watcher(
    dex: Dex,
    dex_tx: watch::Sender<f64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            ticker.tick().await;
            if let Ok(price) = dex.fetch_price_usdc_per_eth().await {
                let _ = dex_tx.send(price);
            }
        }
    })
}

/// Initialize pool state watcher
pub async fn init_pool_state_watcher(
    dex: &Dex,
    _pool_tx: watch::Sender<PoolState>,
) -> anyhow::Result<watch::Receiver<PoolState>> {
    // Get initial pool state
    let initial_state = dex.get_pool_state(18, 6, None, None).await?;
    let (tx, rx) = watch::channel(initial_state);

    // Spawn background task to update pool state
    let dex_clone = dex.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            ticker.tick().await;
            if let Ok(state) = dex_clone.get_pool_state(18, 6, None, None).await {
                let _ = tx.send(state);
            }
        }
    });

    Ok(rx)
}

pub use calc::{SwapDirection, SwapResult, calculate_swap};
pub use state::PoolState;

abigen!(
    UniswapV3Pool,
    r#"[
        function slot0() view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)
        function liquidity() view returns (uint128)
        function fee() view returns (uint24)
        function tickSpacing() view returns (int24)
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

        Ok(PoolState::new(
            sqrt_price_x96,
            liquidity,
            tick as i32,
            token0_decimals,
            token1_decimals,
            lower_q96,
            upper_q96,
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
        Ok(Self::price_usdc_per_eth(sqrt_price_x96))
    }

    /// Convert sqrtPriceX96 to USDC per ETH price
    pub fn price_usdc_per_eth(sqrt_price_x96: U256) -> f64 {
        let sqrt_price = sqrt_price_x96.as_u128() as f64 / 2.0_f64.powi(96);
        let price = sqrt_price * sqrt_price;

        // From debug output: sqrt_price=15418, price=237724591
        // This means sqrtPriceX96 = sqrt(ETH/USDC) in Q96
        // We want USDC/ETH, so take the inverse
        // But price is in raw units, need to adjust for decimals
        (1.0 / price) * 10_f64.powi(12)
    }
}
