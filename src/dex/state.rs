use ethers::types::U256;

/// Minimal immutable snapshot of a Uniswap V3 pool state needed for pricing
/// and swap sizing within a single tick.
#[derive(Clone, Debug)]
pub struct PoolState {
    /// Current sqrt(price1/price0) in Q96 (Uniswap V3 `slot0.sqrtPriceX96`).
    pub sqrt_price_x96: U256,
    /// Current in-range liquidity L (Uniswap V3 `liquidity()`), raw uint128 value.
    pub liquidity: u128,
    /// Current tick index (Uniswap V3 `slot0.tick`).
    pub tick: i32,
    /// Token0 decimals (e.g., WETH 18)
    pub token0_decimals: u8,
    /// Token1 decimals (e.g., USDC 6)
    pub token1_decimals: u8,
    /// Lower and upper sqrt price limits of the current tick, if known.
    pub limit_lower_sqrt_price_x96: Option<U256>,
    pub limit_upper_sqrt_price_x96: Option<U256>,
}

impl PoolState {
    pub fn new(
        sqrt_price_x96: U256,
        liquidity: u128,
        tick: i32,
        token0_decimals: u8,
        token1_decimals: u8,
        limit_lower_sqrt_price_x96: Option<U256>,
        limit_upper_sqrt_price_x96: Option<U256>,
    ) -> Self {
        Self {
            sqrt_price_x96,
            liquidity,
            tick,
            token0_decimals,
            token1_decimals,
            limit_lower_sqrt_price_x96,
            limit_upper_sqrt_price_x96,
        }
    }
}

/// Approximate sqrtPriceX96 at a given tick using f64 math.
/// This is a lightweight alternative to the exact TickMath and is sufficient
/// for bounding the current tick segment.
pub fn approx_sqrt_price_x96_at_tick(tick: i32) -> U256 {
    // sqrt(1.0001^tick) = 1.0001^(tick/2)
    let pow = (1.0001f64).powf(tick as f64 / 2.0);
    let two_pow_96_f = (2f64).powi(96);
    let value = pow * two_pow_96_f;
    // Clamp to non-negative and convert to decimal string then U256
    let s = if value.is_finite() && value > 0.0 {
        format!("{:.0}", value)
    } else {
        "0".to_string()
    };
    U256::from_dec_str(&s).unwrap_or_else(|_| U256::zero())
}
