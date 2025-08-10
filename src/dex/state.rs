use bigdecimal::{BigDecimal, One, Zero};
use ethers::types::U256;
use num_bigint::{BigInt, ToBigInt};
use std::str::FromStr;

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
    /// Piecewise segments for multi-tick calculations (down = decreasing S, up = increasing S).
    pub segments_down: Vec<PriceSegment>,
    pub segments_up: Vec<PriceSegment>,
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
        segments_down: Vec<PriceSegment>,
        segments_up: Vec<PriceSegment>,
    ) -> Self {
        Self {
            sqrt_price_x96,
            liquidity,
            tick,
            token0_decimals,
            token1_decimals,
            limit_lower_sqrt_price_x96,
            limit_upper_sqrt_price_x96,
            segments_down,
            segments_up,
        }
    }
}

/// Placeholder for costs; not used yet in calculations but provided so
/// integration of fees/gas can be added without touching core math.
#[derive(Clone, Copy, Debug, Default)]
pub struct TradeCosts {
    /// e.g., 3.0 for 3 bps = 0.03%
    pub dex_fee_bps: f64,
    /// e.g., 10.0 for 10 bps = 0.10%
    pub cex_fee_bps: f64,
    /// Gas cost expressed in token1 units (e.g., USDC).
    pub gas_cost_token1: f64,
}

/// A price range with constant liquidity. For `segments_down`,
/// `start_sqrt_price_x96 > end_sqrt_price_x96`. For `segments_up`,
/// `start_sqrt_price_x96 < end_sqrt_price_x96`.
#[derive(Clone, Debug)]
pub struct PriceSegment {
    pub start_sqrt_price_x96: U256,
    pub end_sqrt_price_x96: U256,
    pub liquidity: u128,
}

// ---------- helpers to compute sqrt ratios and conversions ----------

pub fn q96_to_real(q96: &U256) -> BigDecimal {
    let two_pow_96 = BigInt::one() << 96u32;
    let num = BigInt::from_str(&q96.to_string()).unwrap_or_else(|_| BigInt::from(0));
    let num_bd = BigDecimal::from(num);
    let den_bd = BigDecimal::from(two_pow_96);
    num_bd / den_bd
}

pub fn real_to_q96(real: &BigDecimal) -> U256 {
    let two_pow_96 = BigInt::one() << 96u32;
    let scaled = real * BigDecimal::from(two_pow_96);
    let scaled_int: BigInt = scaled
        .with_scale(0)
        .to_bigint()
        .unwrap_or_else(|| BigInt::from(0));
    U256::from_dec_str(&scaled_int.to_string()).unwrap_or_else(|_| U256::zero())
}

pub fn reciprocal(x: &BigDecimal) -> BigDecimal {
    if x.is_zero() {
        return BigDecimal::zero();
    }
    BigDecimal::one() / x.clone()
}

/// Approximate sqrtPriceX96 at a given tick using f64 math.
/// This is a lightweight alternative to the exact TickMath and is sufficient
/// for bounding the current tick segment. For precise boundary math, port the
/// exact Uniswap V3 TickMath constants.
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
