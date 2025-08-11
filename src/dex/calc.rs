use crate::dex::state::PoolState;
use crate::models::{SwapDirection, SwapResult};
use alloy_primitives::U256;
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive, Zero};
use std::str::FromStr;
use uniswap_v3_math::{
    error::UniswapV3MathError,
    sqrt_price_math::{_get_amount_0_delta, _get_amount_1_delta},
};

/// Calculate swap using Uniswap V3 math library with high precision
/// This function calculates the optimal swap amounts to reach a target price
/// using rational math to avoid f64 precision loss in price calculations.
pub fn calculate_swap_with_library(
    pool: &PoolState,
    target_price: f64,
    direction: SwapDirection,
    fee_bps: f64,
    max_amount: f64,
) -> Result<SwapResult, UniswapV3MathError> {
    // Convert current sqrtPriceX96 to U256
    let sqrt_price_start = U256::from_str_radix(&pool.sqrt_price_x96.to_string(), 10)
        .map_err(|_| UniswapV3MathError::SqrtPriceIsZero)?;

    // Convert liquidity to u128
    let liquidity = pool.liquidity;

    // Calculate amounts using library functions
    let (amount_in, amount_out) = match direction {
        SwapDirection::Token0ToToken1 => {
            // USDC in, ETH out (price UP). Human price up
            // CEX price > DEX price: buy ETH on DEX to profit

            // Calculate target sqrt price using BigDecimal for precision
            let real_target_price = target_price * (1.0 - fee_bps / 10_000.0);
            let sqrt_price_target = calculate_sqrt_price_with_precision_per_eth(
                real_target_price,
                pool.token0_decimals,
                pool.token1_decimals,
            )?;
            if sqrt_price_target >= sqrt_price_start {
                return Ok(SwapResult {
                    amount_in: 0.0,
                    amount_out: 0.0,
                    hit_boundary: false,
                });
            }

            let amount0_in = _get_amount_0_delta(
                sqrt_price_start,
                sqrt_price_target,
                liquidity,
                true, // round up
            )?;

            let amount1_out = _get_amount_1_delta(
                sqrt_price_start,
                sqrt_price_target,
                liquidity,
                false, // round down
            )?;

            // Apply fee: Uniswap V3 applies fee to input amount
            // amount_in_with_fee = amount_in / (1 - fee_fraction)
            let fee_fraction = BigDecimal::from_f64(fee_bps / 10_000.0)
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
            let one_minus_fee = BigDecimal::from_f64(1.0).unwrap() - fee_fraction;

            let amount0_in_bd = BigDecimal::from_u128(amount0_in.try_into().unwrap_or(0u128))
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
            let amount0_in_with_fee = (amount0_in_bd / one_minus_fee)
                .to_f64()
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;

            (
                amount0_in_with_fee,
                amount1_out.try_into().unwrap_or(0u128) as f64,
            )
        }
        SwapDirection::Token1ToToken0 => {
            // ETH in, USDC out (price DOWN). Human price down => sqrt increases.
            // CEX price < DEX price: sell ETH on DEX to profit
            let fee_bps_adjusted = fee_bps / 10_000.0;
            // We are selling ETH, so we need to increase the price by the fee to adjust our target
            let real_target_price = target_price / (1.0 - fee_bps_adjusted);
            let sqrt_price_target = calculate_sqrt_price_with_precision_per_eth(
                real_target_price,
                pool.token0_decimals,
                pool.token1_decimals,
            )?;
            if sqrt_price_target <= sqrt_price_start {
                return Ok(SwapResult {
                    amount_in: 0.0,
                    amount_out: 0.0,
                    hit_boundary: false,
                });
            }

            let amount1_in = _get_amount_1_delta(
                sqrt_price_target,
                sqrt_price_start,
                liquidity,
                true, // round up
            )?;

            let amount0_out = _get_amount_0_delta(
                sqrt_price_target,
                sqrt_price_start,
                liquidity,
                false, // round down
            )?;

            // include fee to amount1_in
            // amount_1_in = x * (1 - fee_bps_adjusted)
            // x = amount_1_in / (1 - fee_bps_adjusted)
            let amount1_in_bd = BigDecimal::from_u128(amount1_in.try_into().unwrap_or(0u128))
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
            let fee_fraction_bd = BigDecimal::from_f64(fee_bps_adjusted)
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
            let one_minus_fee_adjusted = BigDecimal::from_f64(1.0).unwrap() - fee_fraction_bd;
            let amount1_in_with_fee = (amount1_in_bd / one_minus_fee_adjusted)
                .to_f64()
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;

            (
                amount1_in_with_fee,
                amount0_out.try_into().unwrap_or(0u128) as f64,
            )
        }
    };

    // Cap by max_amount if needed
    let mut final_amount_in = amount_in; // RAW units
    let mut final_amount_out = amount_out; // RAW units

    // Convert human max_amount to RAW units for the input token
    let max_in_raw: f64 = match direction {
        // Token0ToToken1: input is token0 (USDC), 6 decimals
        SwapDirection::Token0ToToken1 => {
            let scale = 10f64.powi(pool.token0_decimals as i32);
            max_amount * scale
        }
        // Token1ToToken0: input is token1 (ETH), 18 decimals
        SwapDirection::Token1ToToken0 => {
            let scale = 10f64.powi(pool.token1_decimals as i32);
            max_amount * scale
        }
    };

    if amount_in > max_in_raw {
        let scale = max_in_raw / amount_in;
        final_amount_in = max_in_raw;
        final_amount_out = amount_out * scale;
    }

    // Convert RAW amounts to human units
    let (final_in_human, final_out_human) = match direction {
        SwapDirection::Token0ToToken1 => {
            let in_scale = 10f64.powi(pool.token0_decimals as i32);
            let out_scale = 10f64.powi(pool.token1_decimals as i32);
            (final_amount_in / in_scale, final_amount_out / out_scale)
        }
        SwapDirection::Token1ToToken0 => {
            let in_scale = 10f64.powi(pool.token1_decimals as i32);
            let out_scale = 10f64.powi(pool.token0_decimals as i32);
            (final_amount_in / in_scale, final_amount_out / out_scale)
        }
    };

    // Calculate execution price directly from human units
    let _execution_price = match direction {
        SwapDirection::Token0ToToken1 => {
            if final_out_human > 0.0 {
                final_in_human / final_out_human
            } else {
                0.0
            }
        }
        SwapDirection::Token1ToToken0 => {
            if final_in_human > 0.0 {
                final_out_human / final_in_human
            } else {
                0.0
            }
        }
    };

    Ok(SwapResult {
        amount_in: final_in_human,
        amount_out: final_out_human,
        hit_boundary: false,
    })
}

/// Calculate sqrt price using BigDecimal for high precision
///
/// Converts a human-readable price to sqrtPriceX96
fn calculate_sqrt_price_with_precision_per_eth(
    price: f64,
    token0_decimals: u8,
    token1_decimals: u8,
) -> Result<U256, UniswapV3MathError> {
    if price <= 0.0 {
        return Err(UniswapV3MathError::SqrtPriceIsZero);
    }

    // Calculate decimals factor: 10^(token1_decimals - token0_decimals)
    let decimals_diff = token1_decimals as i32 - token0_decimals as i32;
    let decimals_factor_f64 = 10.0_f64.powi(decimals_diff);
    let decimals_factor =
        BigDecimal::from_f64(decimals_factor_f64).ok_or(UniswapV3MathError::SqrtPriceIsZero)?;

    // Calculate ratio: decimals_factor / target_price
    let price_bd = BigDecimal::from_f64(price).ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
    let ratio = decimals_factor / price_bd;

    // Calculate sqrt of ratio using f64 for better compatibility
    let ratio_f64 = ratio.to_f64().ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
    let sqrt_ratio_f64 = ratio_f64.sqrt();

    if sqrt_ratio_f64.is_nan() || sqrt_ratio_f64 <= 0.0 {
        return Err(UniswapV3MathError::SqrtPriceIsZero);
    }

    // Multiply by 2^96 to get Q96 format
    let two_pow_96_f64 = 2.0_f64.powi(96);
    let sqrt_price_q96_f64 = sqrt_ratio_f64 * two_pow_96_f64;

    // Convert to U256 using string conversion for precision
    let sqrt_price_str = format!("{:.0}", sqrt_price_q96_f64);
    U256::from_str_radix(&sqrt_price_str, 10).map_err(|_| UniswapV3MathError::SqrtPriceIsZero)
}

/// Calculate human-readable price from sqrtPriceX96
///
/// Converts sqrtPriceX96 back to human-readable price (USDC per ETH)
/// for debugging and logging purposes.
fn calculate_human_price_from_sqrt_x96(
    sqrt_price_x96: U256,
    token0_decimals: u8,
    token1_decimals: u8,
) -> f64 {
    let sqrt_price_str = sqrt_price_x96.to_string();
    let sqrt_price_bd =
        BigDecimal::from_str(&sqrt_price_str).unwrap_or_else(|_| BigDecimal::zero());

    // Divide by 2^96 to get sqrt ratio
    let two_pow_96_f64 = 2.0_f64.powi(96);
    let two_pow_96 = BigDecimal::from_f64(two_pow_96_f64).unwrap();
    let sqrt_ratio = sqrt_price_bd / two_pow_96;

    // Square to get ratio
    let ratio = &sqrt_ratio * &sqrt_ratio;

    // Calculate price: decimals_factor / ratio
    let decimals_diff = token1_decimals as i32 - token0_decimals as i32;
    let decimals_factor_f64 = 10.0_f64.powi(decimals_diff);
    let decimals_factor = BigDecimal::from_f64(decimals_factor_f64).unwrap();
    let price_bd = decimals_factor / ratio;

    price_bd.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dex::state::PoolState;

    fn make_pool(price_usdc_per_eth: f64, liquidity: u128) -> PoolState {
        let token0_decimals = 6; // USDC
        let token1_decimals = 18; // WETH
        let sqrt_price_x96 = calculate_sqrt_price_with_precision_per_eth(
            price_usdc_per_eth,
            token0_decimals,
            token1_decimals,
        )
        .unwrap();
        PoolState {
            sqrt_price_x96,
            liquidity,
            tick: 0,
            token0_decimals,
            token1_decimals,
            limit_lower_sqrt_price_x96: None,
            limit_upper_sqrt_price_x96: None,
            price_usdc_per_eth,
        }
    }

    #[test]
    fn test_calculate_sqrt_price_with_precision() {
        let price = 9.0;
        let sqrt_price = calculate_sqrt_price_with_precision_per_eth(price, 6, 18).unwrap();
        let price_usdc_per_eth = calculate_human_price_from_sqrt_x96(sqrt_price, 6, 18);
        // Use approximate equality due to floating-point precision
        let tolerance = 1e-10;
        assert!(
            (price_usdc_per_eth - price).abs() < tolerance,
            "Expected price {} to be within {} of {}",
            price_usdc_per_eth,
            tolerance,
            price
        );
    }

    #[test]
    fn direction_a_profitable_when_dex_below_cex_no_fee() {
        let pool = make_pool(4223.0, 1_800_000_000_000_000_000); // ~1.8e18
        let bid_price = 4225.0; // CEX bid above DEX
        let res = calculate_swap_with_library(
            &pool,
            bid_price,
            SwapDirection::Token0ToToken1,
            0.0,
            10_000.0,
        )
        .unwrap();
        assert!(res.amount_in > 0.0);
        assert!(res.amount_out > 0.0);
    }

    #[test]
    fn direction_b_profitable_when_dex_above_cex_no_fee() {
        let pool = make_pool(4225.0, 1_800_000_000_000_000_000);
        let ask_price = 4223.0; // CEX ask below DEX
        let res =
            calculate_swap_with_library(&pool, ask_price, SwapDirection::Token1ToToken0, 0.0, 5.0)
                .unwrap();
        assert!(res.amount_in > 0.0);
        assert!(res.amount_out > 0.0);
    }

    #[test]
    fn direction_a_profitable_when_dex_below_cex_with_fee() {
        let pool = make_pool(4000.0, 1_800_000_000_000_000_000);
        let bid_price = 4250.0; // CEX bid above DEX
        // price diff is 250/4250 = 5.88%
        let res = calculate_swap_with_library(
            &pool,
            bid_price,
            SwapDirection::Token0ToToken1,
            588.0,
            10_000.0,
        )
        .unwrap();
        assert!(res.amount_in > 0.0);
        assert!(res.amount_out > 0.0);

        // Check where fee is 6.26%
        let res = calculate_swap_with_library(
            &pool,
            bid_price,
            SwapDirection::Token0ToToken1,
            589.0,
            10_000.0,
        )
        .unwrap();
        assert!(res.amount_in <= 0.0);
        assert!(res.amount_out <= 0.0);
    }

    #[test]
    fn caps_max_input_and_scales_output() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        let price = 4210.0;
        let res =
            calculate_swap_with_library(&pool, price, SwapDirection::Token0ToToken1, 0.0, 0.5)
                .unwrap();
        assert!(res.amount_in <= 0.5 + 1e-9);
    }
}
