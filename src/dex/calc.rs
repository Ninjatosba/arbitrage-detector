use crate::dex::state::PoolState;

#[derive(Debug, Clone)]
pub struct SwapResult {
    pub amount_in: f64,
    pub amount_out: f64,
    pub execution_price: f64,
    pub hit_boundary: bool,
    pub capped_by_max: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapDirection {
    /// token0 (USDC) in  → token1 (WETH) out → price UP  → √P increases
    Token0ToToken1,
    /// token1 (WETH) in → token0 (USDC) out → price DOWN → √P decreases
    Token1ToToken0,
}

/// Calculate swap amounts using V3 math (single tick only)
///
/// # Arguments
/// * `pool` - Pool state with current price and liquidity
/// * `target_price` - Target price for the swap
/// * `direction` - Swap direction
/// * `fee_bps` - Pool fee in basis points (e.g., 30 for 0.3%)
/// * `max_amount` - Maximum amount limit (input or output)
///
/// # Returns
/// Returns the amount of token0 or token1 needed to swap to get the target price.
/// If the swap is not possible, returns 0.0.
pub fn calculate_swap(
    pool: &PoolState,
    target_price: f64,
    direction: SwapDirection,
    fee_bps: f64,
    max_amount: f64,
) -> SwapResult {
    // Convert Q96 sqrtPriceX96 to f64 sqrt price (sqrt(token1/token0) in raw units)
    let sqrt_price_start = q96_to_f64(&pool.sqrt_price_x96);

    // Target price is expected in *decimal* terms: token0 per token1 (e.g. USDC per ETH).
    // Uniswap math, however, works with the raw ratio token1/token0 in nominal units.
    // token1/token0  =  (1 / price_token0_per_token1) * 10^(dec0 - dec1)
    let decimals_factor = 10_f64.powi(pool.token0_decimals as i32 - pool.token1_decimals as i32); // 10^(18-6)=1e12
    // ratio_raw = price / 10^12
    let ratio_target_raw = if target_price > 0.0 {
        decimals_factor / target_price
    } else {
        0.0
    };

    let sqrt_price_target = f64::sqrt(ratio_target_raw);

    // Clamp target sqrt within current tick boundaries if provided
    let mut effective_sqrt_target = sqrt_price_target;
    let mut hit_boundary = false;
    match direction {
        // price UP → sqrt increases → clamp to UPPER boundary
        SwapDirection::Token0ToToken1 => {
            if let Some(upper_q96) = pool.limit_upper_sqrt_price_x96 {
                let upper = q96_to_f64(&upper_q96);
                if upper > 0.0 && effective_sqrt_target > upper {
                    effective_sqrt_target = upper;
                    hit_boundary = true;
                }
            }
        }
        // price DOWN → sqrt decreases → clamp to LOWER boundary
        SwapDirection::Token1ToToken0 => {
            if let Some(lower_q96) = pool.limit_lower_sqrt_price_x96 {
                let lower = q96_to_f64(&lower_q96);
                if lower > 0.0 && effective_sqrt_target < lower {
                    effective_sqrt_target = lower;
                    hit_boundary = true;
                }
            }
        }
    }

    // Liquidity as f64
    let liquidity_f64 = pool.liquidity as f64;

    // Fee in decimal form (basis points → fraction)
    let fee_fraction = fee_bps / 10_000.0;

    // Basic sanity checks
    if sqrt_price_start <= 0.0 || sqrt_price_target <= 0.0 || target_price <= 0.0 {
        return SwapResult {
            amount_in: 0.0,
            amount_out: 0.0,
            execution_price: 0.0,
            hit_boundary: false,
            capped_by_max: false,
        };
    }

    // Adjust direction validation after clamping
    match direction {
        // USDC in, price UP ⇒ sqrt_target must be > start
        SwapDirection::Token0ToToken1 => {
            if effective_sqrt_target <= sqrt_price_start {
                return SwapResult {
                    amount_in: 0.0,
                    amount_out: 0.0,
                    execution_price: 0.0,
                    hit_boundary: false,
                    capped_by_max: false,
                };
            }
        }
        // WETH in, price DOWN ⇒ sqrt_target must be < start
        SwapDirection::Token1ToToken0 => {
            if effective_sqrt_target >= sqrt_price_start {
                return SwapResult {
                    amount_in: 0.0,
                    amount_out: 0.0,
                    execution_price: 0.0,
                    hit_boundary: false,
                    capped_by_max: false,
                };
            }
        }
    }

    // Main calculation
    let (amount_in, amount_out) = match direction {
        // token0 -> token1 (USDC in, price UP)
        SwapDirection::Token0ToToken1 => {
            let amount0_in_no_fee =
                liquidity_f64 * (1.0 / sqrt_price_start - 1.0 / effective_sqrt_target);
            let amount1_out = liquidity_f64 * (sqrt_price_start - effective_sqrt_target).max(0.0);

            let amount0_in_with_fee = amount0_in_no_fee / (1.0 - fee_fraction);

            (amount0_in_with_fee, amount1_out)
        }
        // token1 -> token0 (ETH in, price DOWN)
        SwapDirection::Token1ToToken0 => {
            let amount1_in_no_fee = liquidity_f64 * (sqrt_price_start - effective_sqrt_target);
            let amount0_out =
                liquidity_f64 * (1.0 / effective_sqrt_target - 1.0 / sqrt_price_start);

            let amount1_in_with_fee = amount1_in_no_fee / (1.0 - fee_fraction);

            (amount1_in_with_fee, amount0_out)
        }
    };

    // Cap by max_amount if needed
    let mut capped_by_max = false;
    let mut final_amount_in = amount_in;
    let mut final_amount_out = amount_out;
    if amount_in > max_amount {
        capped_by_max = true;
        // Scale down proportionally
        let scale = max_amount / amount_in;
        final_amount_in = max_amount;
        final_amount_out = amount_out * scale;
    }

    // Execution price: quote in USDC per ETH regardless of direction
    let execution_price = match direction {
        // USDC in, ETH out: price = USDC / ETH = amount0_in / amount1_out
        SwapDirection::Token0ToToken1 => {
            if final_amount_out > 0.0 {
                final_amount_in / final_amount_out
            } else {
                0.0
            }
        }
        // WETH in, USDC out: price = USDC / ETH = amount0_out / amount1_in
        SwapDirection::Token1ToToken0 => {
            if final_amount_out > 0.0 {
                final_amount_out / final_amount_in
            } else {
                0.0
            }
        }
    };

    SwapResult {
        amount_in: final_amount_in,
        amount_out: final_amount_out,
        execution_price,
        hit_boundary,
        capped_by_max,
    }
}

// ---------- helper functions ----------

fn q96_to_f64(q96: &ethers::types::U256) -> f64 {
    // Convert full 256-bit integer to decimal string then to f64 to avoid truncation
    // Then divide by 2^96 to get the floating sqrt price
    let s = q96.to_string();
    let int_val = s.parse::<f64>().unwrap_or(0.0);
    let two_pow_96 = 2.0_f64.powi(96);
    int_val / two_pow_96
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::U256;

    fn mock_pool(
        sqrt_price: f64,
        liquidity: u128,
        token0_decimals: u8,
        token1_decimals: u8,
    ) -> PoolState {
        // Convert sqrt_price (f64) into Q96 representation
        let q96_val = (sqrt_price * 2.0_f64.powi(96)) as u128;
        PoolState {
            sqrt_price_x96: U256::from(q96_val),
            liquidity,
            tick: 0,
            token0_decimals,
            token1_decimals,
            limit_lower_sqrt_price_x96: None,
            limit_upper_sqrt_price_x96: None,
        }
    }

    #[test]
    fn token0_to_token1_simple_price_drop_same_decimals() {
        // Start price = 1.21 (sqrt=1.1)
        let s_start = 1.1_f64;
        let pool = mock_pool(s_start, 1_000_000, 6, 6);

        let target_price = 1.0;
        let fee_bps = 30.0;
        let max_amount = f64::MAX;

        let res = calculate_swap(
            &pool,
            target_price,
            SwapDirection::Token0ToToken1,
            fee_bps,
            max_amount,
        );

        println!("{:?}", res);
        assert!(res.amount_in > 0.0);
        assert!(res.amount_out > 0.0);
        assert!(res.execution_price > 0.0);
        assert!(!res.capped_by_max);
    }

    #[test]
    fn token1_to_token0_simple_price_rise_different_decimals() {
        // Start price = 1.0 (sqrt=1.0)
        let s_start = 1.0_f64;
        // token0 decimals 18, token1 decimals 6
        let pool = mock_pool(s_start, 1_000_000, 18, 6);

        let target_price = 1.21;
        let fee_bps = 30.0;
        let max_amount = f64::MAX;

        let res = calculate_swap(
            &pool,
            target_price,
            SwapDirection::Token1ToToken0,
            fee_bps,
            max_amount,
        );

        println!("{:?}", res);
        assert!(res.amount_in > 0.0);
        assert!(res.amount_out > 0.0);
        assert!(res.execution_price > 0.0);
        assert!(!res.capped_by_max);
    }

    #[test]
    fn respects_max_amount_cap() {
        let s_start = 1.0_f64;
        let pool = mock_pool(s_start, 1_000_000, 18, 6);

        let target_price = 0.8;
        let fee_bps = 30.0;
        let max_amount = 10.0; // very small cap

        let res = calculate_swap(
            &pool,
            target_price,
            SwapDirection::Token0ToToken1,
            fee_bps,
            max_amount,
        );

        println!("{:?}", res);
        assert!(res.capped_by_max);
        assert_eq!(res.amount_in, max_amount);
        assert!(res.amount_out > 0.0);
    }

    #[test]
    fn invalid_direction_returns_zero() {
        let s_start = 1.0_f64;
        let pool = mock_pool(s_start, 1_000_000, 18, 6);

        // Target price higher than start for Token0ToToken1 (invalid)
        let target_price = 1.5;
        let res = calculate_swap(
            &pool,
            target_price,
            SwapDirection::Token0ToToken1,
            30.0,
            f64::MAX,
        );

        println!("{:?}", res);
        assert_eq!(res.amount_in, 0.0);
        assert_eq!(res.amount_out, 0.0);
        assert_eq!(res.execution_price, 0.0);
    }

    #[test]
    fn zero_or_negative_price_returns_zero() {
        let s_start = 1.0_f64;
        let pool = mock_pool(s_start, 1_000_000, 6, 6);

        let res = calculate_swap(&pool, 0.0, SwapDirection::Token0ToToken1, 30.0, f64::MAX);
        assert_eq!(res.amount_in, 0.0);
        assert_eq!(res.amount_out, 0.0);

        let res2 = calculate_swap(&pool, -1.0, SwapDirection::Token1ToToken0, 30.0, f64::MAX);
        assert_eq!(res2.amount_in, 0.0);
        assert_eq!(res2.amount_out, 0.0);
    }
}
