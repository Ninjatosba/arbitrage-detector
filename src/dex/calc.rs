use crate::dex::state::{PoolState, TradeCosts, q96_to_real, real_to_q96, reciprocal};
use bigdecimal::{BigDecimal, Zero};
use ethers::types::U256;
use num_bigint::BigInt;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct SolveToPriceResult {
    /// Token0 amount to sell on the DEX, in human units (e.g., ETH).
    pub amount_token0_in: BigDecimal,
    /// Token1 amount expected out from the DEX, in human units (e.g., USDC).
    pub amount_token1_out: BigDecimal,
    /// Token1 amount input to the DEX (when buying token0 on DEX), in human units.
    pub amount_token1_in: BigDecimal,
    /// Token0 amount output from the DEX (when buying token0 on DEX), in human units.
    pub amount_token0_out: BigDecimal,
    /// Ending sqrtPriceX96 after executing the trade (or boundary/cap), Q96 integer.
    pub end_sqrt_price_x96: U256,
    /// Realized average price token1 per token0 (human units), i.e., amount_token1_out/amount_token0_in.
    pub realized_avg_price_t1_per_t0: BigDecimal,
    /// True if computation was truncated by the next tick boundary.
    pub hit_tick_boundary: bool,
    /// True if computation was truncated by the provided CEX cap.
    pub capped_by_cex: bool,
    /// True if the target price requires the opposite swap direction (no token0-in solution).
    pub direction_mismatch: bool,
}

impl Default for SolveToPriceResult {
    fn default() -> Self {
        Self {
            amount_token0_in: BigDecimal::zero(),
            amount_token1_out: BigDecimal::zero(),
            amount_token1_in: BigDecimal::zero(),
            amount_token0_out: BigDecimal::zero(),
            end_sqrt_price_x96: U256::zero(),
            realized_avg_price_t1_per_t0: BigDecimal::zero(),
            hit_tick_boundary: false,
            capped_by_cex: false,
            direction_mismatch: false,
        }
    }
}

/// Computes how much token0 to sell on the DEX so that the average execution price
/// matches the given target price (token1 per token0, in human units), under the
/// assumption the swap remains within the current tick (constant liquidity).
///
/// - If the desired move would cross the next tick, the trade is truncated to the
///   tick boundary and `hit_tick_boundary` is set.
/// - If `max_token0_in_human` is provided and the required amount exceeds it, the
///   trade is truncated to the cap and `capped_by_cex` is set.
/// - If the target price is above or equal to the current price, there is no
///   token0->token1 solution within the current tick; `direction_mismatch` is set
///   and zero amounts are returned.
///
/// This is a pure function: it does not query chain state. Costs are provided for
/// future extension and are currently not applied to the math.
pub fn solve_token0_in_for_target_avg_price(
    pool: &PoolState,
    target_price_t1_per_t0_human: &BigDecimal,
    max_token0_in_human: Option<&BigDecimal>,
    _costs: Option<TradeCosts>,
) -> SolveToPriceResult {
    // Convert sqrtPriceX96 Q96 to real sqrt price S_a
    let s_a = q96_to_real(&pool.sqrt_price_x96);
    if s_a.is_zero() {
        return SolveToPriceResult::default();
    }

    // Convert target price from human to raw (on-chain units)
    let p_target_raw = human_price_to_raw(
        target_price_t1_per_t0_human,
        pool.token0_decimals,
        pool.token1_decimals,
    );

    // Current raw price P_a = S_a^2
    let p_a_raw = &s_a * &s_a;

    // If target >= current price, selling token0 (which pushes price down) cannot
    // achieve that average; direction mismatch.
    if p_target_raw >= p_a_raw {
        return SolveToPriceResult {
            end_sqrt_price_x96: pool.sqrt_price_x96,
            direction_mismatch: true,
            ..Default::default()
        };
    }

    // Desired end sqrt price S_b so that average price equals target: avg = S_a * S_b
    // => S_b = P_target / S_a
    let s_b_desired = &p_target_raw / &s_a;

    // Apply optional tick boundary (for token0 in, S_b must be >= boundary since S decreases)
    let mut hit_tick_boundary = false;
    let s_b_after_boundary = if let Some(lower_q96) = &pool.limit_lower_sqrt_price_x96 {
        let lower_boundary_s = q96_to_real(lower_q96);
        if s_b_desired < lower_boundary_s {
            hit_tick_boundary = true;
            lower_boundary_s
        } else {
            s_b_desired.clone()
        }
    } else {
        s_b_desired.clone()
    };

    // Liquidity L in raw units
    let l = BigDecimal::from(pool.liquidity);

    // Compute amounts (raw units) for move S_a -> S_b
    // amount0_in = L * (1/S_b - 1/S_a)
    // amount1_out = L * (S_a - S_b)
    let inv_s_a = reciprocal(&s_a);
    let inv_s_b = reciprocal(&s_b_after_boundary);
    let mut amount0_in_raw = &l * (&inv_s_b - &inv_s_a);
    let mut amount1_out_raw = &l * (&s_a - &s_b_after_boundary);

    // Sanity check: if amounts are too large, cap them
    let max_amount0_raw = BigDecimal::from_str("1000000000000000000").unwrap(); // 1 ETH in wei
    let max_amount1_raw = BigDecimal::from_str("1000000000000").unwrap(); // 1M USDC in raw units

    if amount0_in_raw > max_amount0_raw {
        amount0_in_raw = max_amount0_raw.clone();
        // Recalculate amount1_out for capped amount0
        let inv_s_b_cap = (&amount0_in_raw / &l) + &inv_s_a;
        let s_b_cap = reciprocal(&inv_s_b_cap);
        amount1_out_raw = &l * (&s_a - &s_b_cap);
    }

    if amount1_out_raw > max_amount1_raw {
        amount1_out_raw = max_amount1_raw.clone();
        // Recalculate amount0_in for capped amount1
        let s_b_cap = &s_a - (&amount1_out_raw / &l);
        let inv_s_b_cap = reciprocal(&s_b_cap);
        amount0_in_raw = &l * (&inv_s_b_cap - &inv_s_a);
    }

    // Optional CEX cap: limit token0_in (in human units)
    let mut capped_by_cex = false;
    if let Some(cap_human) = max_token0_in_human {
        let cap_raw = to_raw_units(cap_human, pool.token0_decimals);
        if amount0_in_raw > cap_raw {
            capped_by_cex = true;
            // Solve S_b for capped amount0: 1/S_b = amount0/L + 1/S_a
            let inv_s_b_cap = (&cap_raw / &l) + &inv_s_a;
            let s_b_cap = reciprocal(&inv_s_b_cap);
            let inv_s_b_cap = reciprocal(&s_b_cap); // recompute to avoid rounding drift
            amount0_in_raw = &l * (&inv_s_b_cap - &inv_s_a);
            amount1_out_raw = &l * (&s_a - &s_b_cap);
        }
    }

    // Realized average price in raw units: avg = S_a * S_b_effective
    let s_b_effective = if amount0_in_raw.is_zero() {
        s_a.clone()
    } else {
        // Reconstruct S_b_effective from amounts to be consistent
        // amount1_out = L*(S_a - S_b) => S_b = S_a - amount1_out/L
        &s_a - (&amount1_out_raw / &l)
    };
    let avg_price_raw = &s_a * &s_b_effective;

    // Convert amounts and price to human units
    // Use higher precision for token0 size so very small trades are not rounded to zero
    let amount0_in_human = to_human_units(&amount0_in_raw, pool.token0_decimals).with_scale(12);
    let amount1_out_human = to_human_units(&amount1_out_raw, pool.token1_decimals).with_scale(6);
    let avg_price_human =
        raw_price_to_human(&avg_price_raw, pool.token0_decimals, pool.token1_decimals)
            .with_scale(4);

    // Encode end sqrt price as Q96 U256
    let end_q96 = real_to_q96(&s_b_effective);

    SolveToPriceResult {
        amount_token0_in: amount0_in_human,
        amount_token1_out: amount1_out_human,
        amount_token1_in: BigDecimal::zero(),
        amount_token0_out: BigDecimal::zero(),
        end_sqrt_price_x96: end_q96,
        realized_avg_price_t1_per_t0: avg_price_human,
        hit_tick_boundary,
        capped_by_cex,
        direction_mismatch: false,
    }
}

/// Direction of swap for the multi-tick solver
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapDirection {
    Token0ToToken1, // price decreases, S goes down
    Token1ToToken0, // price increases, S goes up
}

/// Multi-tick version: iterate piecewise-constant-liquidity segments until the
/// target average price is reached or caps/boundaries are hit.
/// - `max_input_human` caps the input side: token0 for Token0ToToken1, token1 for Token1ToToken0.
pub fn solve_for_target_avg_price_multi_tick(
    pool: &PoolState,
    target_price_t1_per_t0_human: &BigDecimal,
    direction: SwapDirection,
    max_input_human: Option<&BigDecimal>,
    _costs: Option<TradeCosts>,
) -> SolveToPriceResult {
    let s_a = q96_to_real(&pool.sqrt_price_x96);
    if s_a.is_zero() {
        return SolveToPriceResult::default();
    }

    let p_target_raw = human_price_to_raw(
        target_price_t1_per_t0_human,
        pool.token0_decimals,
        pool.token1_decimals,
    );
    let p_a_raw = &s_a * &s_a;

    // Direction feasibility check
    match direction {
        SwapDirection::Token0ToToken1 if p_target_raw >= p_a_raw => {
            return SolveToPriceResult {
                end_sqrt_price_x96: pool.sqrt_price_x96,
                direction_mismatch: true,
                ..Default::default()
            };
        }
        SwapDirection::Token1ToToken0 if p_target_raw <= p_a_raw => {
            return SolveToPriceResult {
                end_sqrt_price_x96: pool.sqrt_price_x96,
                direction_mismatch: true,
                ..Default::default()
            };
        }
        _ => {}
    }

    // Desired end sqrt price
    let s_b_desired = &p_target_raw / &s_a;

    // Prepare segment traversal
    let mut amount0_in_raw = BigDecimal::from(0u32);
    let mut amount1_in_raw = BigDecimal::from(0u32);
    let mut amount0_out_raw = BigDecimal::from(0u32);
    let mut amount1_out_raw = BigDecimal::from(0u32);
    let mut capped_by_cex = false;
    let mut hit_tick_boundary = false;

    // Helper to handle partial segment based on cap
    let apply_cap_token0_in =
        |l: &BigDecimal, s_from: &BigDecimal, cap_remain_raw: &BigDecimal| -> BigDecimal {
            // 1/S_to = cap/L + 1/S_from
            let inv_s_to = (cap_remain_raw / l) + reciprocal(s_from);
            reciprocal(&inv_s_to)
        };
    let apply_cap_token1_in =
        |l: &BigDecimal, s_from: &BigDecimal, cap_remain_raw: &BigDecimal| -> BigDecimal {
            // S_to = cap/L + S_from
            (cap_remain_raw / l) + s_from
        };

    // First, current tick segment using pool.liquidity and current limits
    let l_current = BigDecimal::from(pool.liquidity);
    let mut s_from = s_a.clone();
    let mut s_to_target = s_b_desired.clone();

    match direction {
        SwapDirection::Token0ToToken1 => {
            if let Some(lower_q96) = &pool.limit_lower_sqrt_price_x96 {
                let lower = q96_to_real(lower_q96);
                if s_to_target < lower {
                    s_to_target = lower.clone();
                    hit_tick_boundary = true;
                }
            }

            // Apply cap if any on token0 input
            if let Some(cap_human) = max_input_human {
                let cap_raw_total = to_raw_units(cap_human, pool.token0_decimals);
                let cap_raw_remain = &cap_raw_total - &amount0_in_raw;
                // full-segment amount0 needed
                let amt0_needed = &l_current * (reciprocal(&s_to_target) - reciprocal(&s_from));
                if amt0_needed > cap_raw_remain {
                    capped_by_cex = true;
                    let s_to_cap = apply_cap_token0_in(&l_current, &s_from, &cap_raw_remain);
                    s_to_target = s_to_cap;
                }
            }

            // Integrate current segment
            let inv_s_from = reciprocal(&s_from);
            let inv_s_to = reciprocal(&s_to_target);
            amount0_in_raw += &l_current * (&inv_s_to - &inv_s_from);
            amount1_out_raw += &l_current * (&s_from - &s_to_target);
            s_from = s_to_target.clone();
        }
        SwapDirection::Token1ToToken0 => {
            if let Some(upper_q96) = &pool.limit_upper_sqrt_price_x96 {
                let upper = q96_to_real(upper_q96);
                if s_to_target > upper {
                    s_to_target = upper.clone();
                    hit_tick_boundary = true;
                }
            }

            // Apply cap if any on token1 input
            if let Some(cap_human) = max_input_human {
                let cap_raw_total = to_raw_units(cap_human, pool.token1_decimals);
                let cap_raw_remain = &cap_raw_total - &amount1_in_raw;
                let amt1_needed = &l_current * (&s_to_target - &s_from);
                if amt1_needed > cap_raw_remain {
                    capped_by_cex = true;
                    let s_to_cap = apply_cap_token1_in(&l_current, &s_from, &cap_raw_remain);
                    s_to_target = s_to_cap;
                }
            }

            amount1_in_raw += &l_current * (&s_to_target - &s_from);
            amount0_out_raw += &l_current * (reciprocal(&s_from) - reciprocal(&s_to_target));
            s_from = s_to_target.clone();
        }
    }

    // If we already reached desired end sqrt, finish
    if (direction == SwapDirection::Token0ToToken1 && s_from <= s_b_desired)
        || (direction == SwapDirection::Token1ToToken0 && s_from >= s_b_desired)
    {
        let end_q96 = real_to_q96(&s_from);
        return finalize_result(
            pool,
            amount0_in_raw,
            amount1_in_raw,
            amount0_out_raw,
            amount1_out_raw,
            end_q96,
            capped_by_cex,
            hit_tick_boundary,
            direction,
        );
    }

    // Traverse subsequent segments
    let segments = match direction {
        SwapDirection::Token0ToToken1 => &pool.segments_down,
        SwapDirection::Token1ToToken0 => &pool.segments_up,
    };

    for seg in segments {
        // segment bounds in real space
        let seg_start = q96_to_real(&seg.start_sqrt_price_x96);
        let seg_end = q96_to_real(&seg.end_sqrt_price_x96);
        let l = BigDecimal::from(seg.liquidity);

        // Limit this segment towards the ultimate target
        let mut s_to = s_b_desired.clone();
        match direction {
            SwapDirection::Token0ToToken1 => {
                // seg_start > seg_end and s moves downwards
                if s_to > seg_start {
                    s_to = seg_start.clone();
                }
                if s_to < seg_end {
                    s_to = seg_end.clone();
                }

                // Apply cap on token0 input
                if let Some(cap_human) = max_input_human {
                    let cap_raw_total = to_raw_units(cap_human, pool.token0_decimals);
                    let cap_raw_remain = &cap_raw_total - &amount0_in_raw;
                    let amt0_needed = &l * (reciprocal(&s_to) - reciprocal(&s_from));
                    if amt0_needed > cap_raw_remain {
                        capped_by_cex = true;
                        let s_to_cap = apply_cap_token0_in(&l, &s_from, &cap_raw_remain);
                        s_to = s_to_cap;
                    }
                }

                // Integrate
                let inv_s_from = reciprocal(&s_from);
                let inv_s_to = reciprocal(&s_to);
                amount0_in_raw += &l * (&inv_s_to - &inv_s_from);
                amount1_out_raw += &l * (&s_from - &s_to);
                s_from = s_to.clone();
            }
            SwapDirection::Token1ToToken0 => {
                // seg_start < seg_end and s moves upwards
                if s_to < seg_start {
                    s_to = seg_start.clone();
                }
                if s_to > seg_end {
                    s_to = seg_end.clone();
                }

                // Apply cap on token1 input
                if let Some(cap_human) = max_input_human {
                    let cap_raw_total = to_raw_units(cap_human, pool.token1_decimals);
                    let cap_raw_remain = &cap_raw_total - &amount1_in_raw;
                    let amt1_needed = &l * (&s_to - &s_from);
                    if amt1_needed > cap_raw_remain {
                        capped_by_cex = true;
                        let s_to_cap = apply_cap_token1_in(&l, &s_from, &cap_raw_remain);
                        s_to = s_to_cap;
                    }
                }

                amount1_in_raw += &l * (&s_to - &s_from);
                amount0_out_raw += &l * (reciprocal(&s_from) - reciprocal(&s_to));
                s_from = s_to.clone();
            }
        }

        // Check if we've reached the desired S_b
        if (direction == SwapDirection::Token0ToToken1 && s_from <= s_b_desired)
            || (direction == SwapDirection::Token1ToToken0 && s_from >= s_b_desired)
        {
            break;
        }
    }

    // Finalize
    let end_q96 = real_to_q96(&s_from);
    finalize_result(
        pool,
        amount0_in_raw,
        amount1_in_raw,
        amount0_out_raw,
        amount1_out_raw,
        end_q96,
        capped_by_cex,
        hit_tick_boundary,
        direction,
    )
}

fn finalize_result(
    pool: &PoolState,
    amount0_in_raw: BigDecimal,
    amount1_in_raw: BigDecimal,
    amount0_out_raw: BigDecimal,
    amount1_out_raw: BigDecimal,
    end_sqrt_q96: U256,
    capped_by_cex: bool,
    hit_tick_boundary: bool,
    direction: SwapDirection,
) -> SolveToPriceResult {
    let amount0_in_h = to_human_units(&amount0_in_raw, pool.token0_decimals);
    let amount1_in_h = to_human_units(&amount1_in_raw, pool.token1_decimals);
    let amount0_out_h = to_human_units(&amount0_out_raw, pool.token0_decimals);
    let amount1_out_h = to_human_units(&amount1_out_raw, pool.token1_decimals);

    let realized_avg = match direction {
        SwapDirection::Token0ToToken1 => {
            if amount0_in_h.is_zero() {
                BigDecimal::from(0u32)
            } else {
                &amount1_out_h / &amount0_in_h
            }
        }
        SwapDirection::Token1ToToken0 => {
            if amount0_out_h.is_zero() {
                BigDecimal::from(0u32)
            } else {
                &amount1_in_h / &amount0_out_h
            }
        }
    };

    SolveToPriceResult {
        amount_token0_in: amount0_in_h,
        amount_token1_out: amount1_out_h,
        amount_token1_in: amount1_in_h,
        amount_token0_out: amount0_out_h,
        end_sqrt_price_x96: end_sqrt_q96,
        realized_avg_price_t1_per_t0: realized_avg,
        hit_tick_boundary,
        capped_by_cex,
        direction_mismatch: false,
    }
}

// ---------- helpers ----------

fn pow10(decimals: u8) -> BigDecimal {
    let ten = BigInt::from(10u32);
    BigDecimal::from(ten.pow(decimals as u32))
}

// moved to state.rs and reused here: q96_to_real, real_to_q96, reciprocal

fn to_raw_units(amount_human: &BigDecimal, decimals: u8) -> BigDecimal {
    amount_human * pow10(decimals)
}

fn to_human_units(amount_raw: &BigDecimal, decimals: u8) -> BigDecimal {
    if decimals == 0 {
        return amount_raw.clone();
    }
    amount_raw / pow10(decimals)
}

fn human_price_to_raw(price_human: &BigDecimal, t0_decimals: u8, t1_decimals: u8) -> BigDecimal {
    // P_human = (token1_human / token0_human)
    // P_raw = P_human * 10^(t1_decimals - t0_decimals)
    // because tokenX_human = tokenX_raw / 10^decimals.
    let scale = if t1_decimals >= t0_decimals {
        pow10(t1_decimals - t0_decimals)
    } else {
        reciprocal(&pow10(t0_decimals - t1_decimals))
    };
    price_human * scale
}

fn raw_price_to_human(price_raw: &BigDecimal, t0_decimals: u8, t1_decimals: u8) -> BigDecimal {
    // Inverse of above
    let scale = if t1_decimals >= t0_decimals {
        reciprocal(&pow10(t1_decimals - t0_decimals))
    } else {
        pow10(t0_decimals - t1_decimals)
    };
    price_raw * scale
}

/// Convenience wrapper: Provide CEX top ask as f64s, return the computed trade sizing.
/// - `cex_top_ask_price_t1_per_t0` is e.g. USDC per ETH
/// - `cex_top_ask_qty_token0` is in token0 human units (e.g., ETH)
pub fn solve_from_cex_top(
    pool: &PoolState,
    cex_top_ask_price_t1_per_t0: f64,
    cex_top_ask_qty_token0: f64,
) -> SolveToPriceResult {
    let price_bd = BigDecimal::from_str(&format!("{}", cex_top_ask_price_t1_per_t0))
        .unwrap_or_else(|_| BigDecimal::zero());
    let qty_bd = BigDecimal::from_str(&format!("{}", cex_top_ask_qty_token0))
        .unwrap_or_else(|_| BigDecimal::zero());
    solve_token0_in_for_target_avg_price(pool, &price_bd, Some(&qty_bd), None)
}

/// Convenience wrapper for the opposite direction (buy on DEX, sell on CEX at bid).
/// Uses the multi-tick solver in Token1->Token0 direction. The input cap is set
/// to `cex_bid_qty_token0 * cex_bid_price_t1_per_t0` (token1 units) so the
/// produced token0 will not exceed the CEX bid size if target is attainable.
pub fn solve_from_cex_bid(
    pool: &PoolState,
    cex_bid_price_t1_per_t0: f64,
    cex_bid_qty_token0: f64,
) -> SolveToPriceResult {
    let price_bd = BigDecimal::from_str(&format!("{}", cex_bid_price_t1_per_t0))
        .unwrap_or_else(|_| BigDecimal::zero());
    let qty_token0_bd = BigDecimal::from_str(&format!("{}", cex_bid_qty_token0))
        .unwrap_or_else(|_| BigDecimal::zero());
    // Approx cap for token1 input when matching the target average
    let cap_token1_bd = &price_bd * &qty_token0_bd;
    solve_for_target_avg_price_multi_tick(
        pool,
        &price_bd,
        SwapDirection::Token1ToToken0,
        Some(&cap_token1_bd),
        None,
    )
}
