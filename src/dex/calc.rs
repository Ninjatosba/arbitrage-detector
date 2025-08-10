use crate::dex::state::PoolState;
use alloy_primitives::U256;
use uniswap_v3_math::{
    error::UniswapV3MathError,
    sqrt_price_math::{_get_amount_0_delta, _get_amount_1_delta},
};

#[derive(Debug, Clone)]
pub struct SwapResult {
    pub amount_in: f64,
    pub amount_out: f64,
    pub hit_boundary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapDirection {
    /// token0 (USDC) in  → token1 (WETH) out → price UP  → √P increases
    Token0ToToken1,
    /// token1 (WETH) in → token0 (USDC) out → price DOWN → √P decreases
    Token1ToToken0,
}
/// Calculate swap using Uniswap V3 math library (more accurate)
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

    // Convert target price to sqrtPriceX96
    let decimals_factor = 10_f64.powi(pool.token1_decimals as i32 - pool.token0_decimals as i32);

    let ratio_target_raw = if target_price > 0.0 {
        decimals_factor / target_price
    } else {
        return Err(UniswapV3MathError::SqrtPriceIsZero);
    };

    let sqrt_price_target = U256::from((ratio_target_raw.sqrt() * 2.0_f64.powi(96)) as u128);

    // Convert liquidity to u128
    let liquidity = pool.liquidity;

    // Calculate amounts using library functions
    let (amount_in, amount_out) = match direction {
        SwapDirection::Token0ToToken1 => {
            // USDC in, ETH out (price UP). Human price up => sqrt decreases.
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

            // Apply fee
            let fee_fraction = fee_bps / 10_000.0;
            let amount0_in_with_fee =
                (amount0_in.try_into().unwrap_or(0u128) as f64) / (1.0 - fee_fraction);
            (
                amount0_in_with_fee,
                amount1_out.try_into().unwrap_or(0u128) as f64,
            )
        }
        SwapDirection::Token1ToToken0 => {
            // ETH in, USDC out (price DOWN). Human price down => sqrt increases.
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

            // Apply fee
            let fee_fraction = fee_bps / 10_000.0;
            let amount1_in_with_fee =
                (amount1_in.try_into().unwrap_or(0u128) as f64) / (1.0 - fee_fraction);

            (
                amount1_in_with_fee,
                amount0_out.try_into().unwrap_or(0u128) as f64,
            )
        }
    };

    // Cap by max_amount if needed
    let mut capped_by_max = false;
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
        capped_by_max = true;
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
    let execution_price = match direction {
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
