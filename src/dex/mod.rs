//! DEX (Uniswap V3) interaction layer.
//!
//! Streams on-chain price data using a Uniswap V3 pool’s `slot0` value.
//! Converts `sqrtPriceX96` to a human-readable USDC/ETH price with high
//! precision via `bigdecimal`.

use std::{str::FromStr, sync::Arc, time::Duration};

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

abigen!(
    UniswapV3Pool,
    r#"[
        function slot0() view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)
    ]"#,
);

/// Handle for interacting with a specific Uniswap V3 pool.
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
