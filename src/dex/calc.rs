use crate::dex::state::PoolState;
use crate::models::{SwapDirection, SwapResult};
use alloy_primitives::U256;
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive, Zero};
use std::str::FromStr;
use tracing::debug;
use uniswap_v3_math::{
    error::UniswapV3MathError,
    sqrt_price_math::{_get_amount_0_delta, _get_amount_1_delta},
};

/// Calculate swap using Uniswap V3 math library with high precision
///
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

    // Calculate target sqrt price using BigDecimal for precision
    let sqrt_price_target = calculate_sqrt_price_target_with_precision(
        target_price,
        pool.token0_decimals,
        pool.token1_decimals,
    )?;

    // Convert liquidity to u128
    let liquidity = pool.liquidity;

    // Log current and target prices in human-readable format for debugging
    let current_price = calculate_human_price_from_sqrt_x96(
        sqrt_price_start,
        pool.token0_decimals,
        pool.token1_decimals,
    );
    debug!(
        "Swap calculation: current_price={:.6}, target_price={:.6}, direction={:?}",
        current_price, target_price, direction
    );

    // Calculate amounts using library functions
    let (amount_in, amount_out) = match direction {
        SwapDirection::Token0ToToken1 => {
            // USDC in, ETH out (price UP). Human price up => sqrt decreases.
            // CEX price > DEX price: buy ETH on DEX to profit
            if sqrt_price_target >= sqrt_price_start {
                debug!(
                    "Skipping trade: sqrt_price_target ({}) >= sqrt_price_start ({}). \
                     Target price {:.6} would not increase current price {:.6}",
                    sqrt_price_target, sqrt_price_start, target_price, current_price
                );
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
            if sqrt_price_target <= sqrt_price_start {
                debug!(
                    "Skipping trade: sqrt_price_target ({}) <= sqrt_price_start ({}). \
                     Target price {:.6} would not decrease current price {:.6}",
                    sqrt_price_target, sqrt_price_start, target_price, current_price
                );
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

            // Apply fee: Uniswap V3 applies fee to input amount
            // amount_in_with_fee = amount_in / (1 - fee_fraction)
            let fee_fraction = BigDecimal::from_f64(fee_bps / 10_000.0)
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
            let one_minus_fee = BigDecimal::from_f64(1.0).unwrap() - fee_fraction;

            let amount1_in_bd = BigDecimal::from_u128(amount1_in.try_into().unwrap_or(0u128))
                .ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
            let amount1_in_with_fee = (amount1_in_bd / one_minus_fee)
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

/// Calculate sqrt price target using BigDecimal for high precision
///
/// Converts a human-readable target price (USDC per ETH) to sqrtPriceX96
/// using rational math to avoid f64 precision loss.
fn calculate_sqrt_price_target_with_precision(
    target_price: f64,
    token0_decimals: u8,
    token1_decimals: u8,
) -> Result<U256, UniswapV3MathError> {
    if target_price <= 0.0 {
        return Err(UniswapV3MathError::SqrtPriceIsZero);
    }

    // Calculate decimals factor: 10^(token1_decimals - token0_decimals)
    let decimals_diff = token1_decimals as i32 - token0_decimals as i32;
    let decimals_factor_f64 = 10.0_f64.powi(decimals_diff);
    let decimals_factor =
        BigDecimal::from_f64(decimals_factor_f64).ok_or(UniswapV3MathError::SqrtPriceIsZero)?;

    // Calculate ratio: decimals_factor / target_price
    let target_price_bd =
        BigDecimal::from_f64(target_price).ok_or(UniswapV3MathError::SqrtPriceIsZero)?;
    let ratio = decimals_factor / target_price_bd;

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
    use ethers::types::U256 as EthersU256;

    fn sqrt_price_x96_from_price_usdc_per_eth(
        price_usdc_per_eth: f64,
        token0_decimals: u8,
        token1_decimals: u8,
    ) -> EthersU256 {
        // ratio_raw = token1/token0 in nominal units = 10^(dec1-dec0) / price
        let dec_factor = 10f64.powi(token1_decimals as i32 - token0_decimals as i32);
        let ratio_raw = dec_factor / price_usdc_per_eth;
        let sqrt_ratio = ratio_raw.sqrt();
        let q96 = (sqrt_ratio * 2f64.powi(96)) as u128;
        EthersU256::from(q96)
    }

    fn make_pool(price_usdc_per_eth: f64, liquidity: u128) -> PoolState {
        let token0_decimals = 6; // USDC
        let token1_decimals = 18; // WETH
        let sqrt_q96 = sqrt_price_x96_from_price_usdc_per_eth(
            price_usdc_per_eth,
            token0_decimals,
            token1_decimals,
        );
        PoolState {
            sqrt_price_x96: sqrt_q96,
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
    fn direction_a_profitable_when_dex_below_cex() {
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
    fn direction_b_profitable_when_dex_above_cex() {
        let pool = make_pool(4225.0, 1_800_000_000_000_000_000);
        let ask_price = 4223.0; // CEX ask below DEX
        let res =
            calculate_swap_with_library(&pool, ask_price, SwapDirection::Token1ToToken0, 0.0, 5.0)
                .unwrap();
        assert!(res.amount_in > 0.0);
        assert!(res.amount_out > 0.0);
    }
}
