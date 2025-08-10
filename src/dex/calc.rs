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
    Token0ToToken1, // ETH → USDC (price decreases)
    Token1ToToken0, // USDC → ETH (price increases)
}

/// Calculate swap amounts using V3 math (single tick only)
///
/// # Arguments
/// * `pool` - Pool state with current price and liquidity
/// * `amount_in` - Input amount in human units
/// * `direction` - Swap direction
/// * `fee_bps` - Pool fee in basis points (e.g., 30 for 0.3%)
/// * `max_amount` - Maximum amount limit (input or output)
///
/// # Returns
/// * `SwapResult` with calculated amounts and execution price
pub fn calculate_swap(
    pool: &PoolState,
    amount_in: f64,
    direction: SwapDirection,
    fee_bps: f64,
    max_amount: f64,
) -> SwapResult {
    // Input validation
    if amount_in <= 0.0 || max_amount <= 0.0 {
        return SwapResult {
            amount_in: 0.0,
            amount_out: 0.0,
            execution_price: 0.0,
            hit_boundary: false,
            capped_by_max: false,
        };
    }

    // Convert sqrtPriceX96 to real sqrt price (simplified)
    let sqrt_price_start = q96_to_f64(&pool.sqrt_price_x96);
    if sqrt_price_start == 0.0 {
        return SwapResult {
            amount_in: 0.0,
            amount_out: 0.0,
            execution_price: 0.0,
            hit_boundary: false,
            capped_by_max: false,
        };
    }

    // Convert to raw units
    let amount_in_raw = to_raw_units(amount_in, pool.token0_decimals);
    let liquidity = pool.liquidity as f64;

    // Calculate new sqrt price and output amount
    let (_sqrt_price_end, amount_out_raw) = match direction {
        SwapDirection::Token0ToToken1 => {
            // ETH → USDC: price decreases, sqrt price decreases
            // amount0_in = L * (1/√P₁ - 1/√P₀)
            // amount1_out = L * (√P₀ - √P₁)

            let inv_sqrt_start = 1.0 / sqrt_price_start;
            let inv_sqrt_end = inv_sqrt_start + (amount_in_raw / liquidity);
            let sqrt_price_end = 1.0 / inv_sqrt_end;

            // Check tick boundary
            let final_sqrt_price = if let Some(lower_q96) = &pool.limit_lower_sqrt_price_x96 {
                let lower_boundary = q96_to_f64(lower_q96);
                if sqrt_price_end < lower_boundary {
                    lower_boundary
                } else {
                    sqrt_price_end
                }
            } else {
                sqrt_price_end
            };

            let amount_out_raw = liquidity * (sqrt_price_start - final_sqrt_price);
            (final_sqrt_price, amount_out_raw)
        }
        SwapDirection::Token1ToToken0 => {
            // USDC → ETH: price increases, sqrt price increases
            // amount1_in = L * (√P₁ - √P₀)
            // amount0_out = L * (1/√P₀ - 1/√P₁)

            let sqrt_price_end = sqrt_price_start + (amount_in_raw / liquidity);

            // Check tick boundary
            let final_sqrt_price = if let Some(upper_q96) = &pool.limit_upper_sqrt_price_x96 {
                let upper_boundary = q96_to_f64(upper_q96);
                if sqrt_price_end > upper_boundary {
                    upper_boundary
                } else {
                    sqrt_price_end
                }
            } else {
                sqrt_price_end
            };

            let inv_sqrt_start = 1.0 / sqrt_price_start;
            let inv_sqrt_end = 1.0 / final_sqrt_price;
            let amount_out_raw = liquidity * (inv_sqrt_start - inv_sqrt_end);
            (final_sqrt_price, amount_out_raw)
        }
    };

    // Convert back to human units
    let amount_out_human = to_human_units(amount_out_raw, pool.token1_decimals);

    // Apply fee
    let fee_multiplier = 1.0 - (fee_bps / 10000.0);
    let amount_out_after_fee = amount_out_human * fee_multiplier;

    // Check max amount limit
    let mut capped_by_max = false;
    let final_amount_out = if amount_out_after_fee > max_amount {
        capped_by_max = true;
        max_amount
    } else {
        amount_out_after_fee
    };

    // Calculate execution price (token1 per token0)
    let execution_price = if amount_in > 0.0 {
        final_amount_out / amount_in
    } else {
        0.0
    };

    SwapResult {
        amount_in,
        amount_out: final_amount_out,
        execution_price,
        hit_boundary: false, // TODO: implement boundary detection
        capped_by_max,
    }
}

// ---------- helper functions ----------

fn q96_to_f64(q96: &ethers::types::U256) -> f64 {
    let two_pow_96 = 2.0_f64.powi(96);
    let num = q96.as_u128() as f64;
    num / two_pow_96
}

fn to_raw_units(amount_human: f64, decimals: u8) -> f64 {
    amount_human * 10_f64.powi(decimals as i32)
}

fn to_human_units(amount_raw: f64, decimals: u8) -> f64 {
    if decimals == 0 {
        amount_raw
    } else {
        amount_raw / 10_f64.powi(decimals as i32)
    }
}
