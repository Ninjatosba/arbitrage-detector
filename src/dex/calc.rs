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
    Token0ToToken1, // ETH → USDC (price decreases)(Selling ETH)
    Token1ToToken0, // USDC → ETH (price increases)(Buying ETH)
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
    // Convert Q96 sqrtPriceX96 to f64 sqrt price
    let sqrt_price_start = q96_to_f64(&pool.sqrt_price_x96);
    let sqrt_price_target = f64::sqrt(target_price);

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

    // Check if target price is in correct direction
    match direction {
        SwapDirection::Token0ToToken1 => {
            if sqrt_price_target >= sqrt_price_start {
                // Selling token0 should push price DOWN.
                return SwapResult {
                    amount_in: 0.0,
                    amount_out: 0.0,
                    execution_price: 0.0,
                    hit_boundary: false,
                    capped_by_max: false,
                };
            }
        }
        SwapDirection::Token1ToToken0 => {
            if sqrt_price_target <= sqrt_price_start {
                // Buying ETH should push price UP.
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
        SwapDirection::Token0ToToken1 => {
            // Selling token0 (price decreases)
            let amount0_in = liquidity_f64 * (1.0 / sqrt_price_target - 1.0 / sqrt_price_start);
            let amount1_out = liquidity_f64 * (sqrt_price_start - sqrt_price_target);

            // Apply fee on input side
            let amount0_in_with_fee = amount0_in / (1.0 - fee_fraction);

            (amount0_in_with_fee, amount1_out)
        }
        SwapDirection::Token1ToToken0 => {
            // Selling token1 (price increases)
            let amount1_in = liquidity_f64 * (sqrt_price_target - sqrt_price_start);
            let amount0_out = liquidity_f64 * (1.0 / sqrt_price_start - 1.0 / sqrt_price_target);

            // Apply fee on input side
            let amount1_in_with_fee = amount1_in / (1.0 - fee_fraction);

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

    // Execution price = amount_in / amount_out in correct token terms
    let execution_price = if final_amount_out > 0.0 {
        final_amount_in / final_amount_out
    } else {
        0.0
    };

    SwapResult {
        amount_in: final_amount_in,
        amount_out: final_amount_out,
        execution_price,
        hit_boundary: false, // For multi-tick handling, you'd set this if you hit tick limit
        capped_by_max,
    }
}

// ---------- helper functions ----------

fn q96_to_f64(q96: &ethers::types::U256) -> f64 {
    let two_pow_96 = 2.0_f64.powi(96);
    let num = q96.as_u128() as f64;
    num / two_pow_96
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
