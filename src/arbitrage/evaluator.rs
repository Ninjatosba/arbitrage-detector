use super::types::{ArbitrageConfig, ArbitrageOpportunity};
use crate::dex::{PoolState, calculate_swap_with_library};
use crate::models::{BookDepth, SwapDirection};

/// Evaluate arbitrage opportunities in both directions
pub fn evaluate_opportunities(
    pool_state: &PoolState,
    book: &BookDepth,
    dex_price: f64,
    config: &ArbitrageConfig,
    gas_cost_usdc: f64,
) -> Vec<ArbitrageOpportunity> {
    let mut opportunities = Vec::new();

    if book.bids.is_empty() || book.asks.is_empty() {
        return opportunities;
    }

    // Direction A: buy on DEX -> sell on CEX (use CEX bid)
    if let Some(opp) = evaluate_direction_a(pool_state, book, config, gas_cost_usdc) {
        opportunities.push(opp);
    }

    // Direction B: buy on CEX -> sell on DEX (use CEX ask)
    if let Some(opp) = evaluate_direction_b(pool_state, book, dex_price, config, gas_cost_usdc) {
        opportunities.push(opp);
    }

    opportunities
}

/// Evaluate Direction A: buy on DEX -> sell on CEX
fn evaluate_direction_a(
    pool_state: &PoolState,
    book: &BookDepth,
    config: &ArbitrageConfig,
    gas_cost_usdc: f64,
) -> Option<ArbitrageOpportunity> {
    let (bid_price, bid_qty_cex) = book.bids[0];
    let effective_bid_price = bid_price * (1.0 - config.cex_fee_bps / 10_000.0);

    let res = calculate_swap_with_library(
        pool_state,
        effective_bid_price,
        SwapDirection::Token0ToToken1,
        config.dex_fee_bps,
        bid_qty_cex,
    )
    .ok()?;

    let mut token1_in = res.amount_in; // USDC we will spend on DEX
    let mut token0_out = res.amount_out; // ETH we obtain from DEX

    // Ensure we don't exceed CEX depth; scale down if necessary
    if token0_out > bid_qty_cex {
        let scale = bid_qty_cex / token0_out;
        token0_out = bid_qty_cex;
        token1_in *= scale;
    }

    if token0_out <= 0.0 {
        return None;
    }

    // Calculate profit and loss: revenue on CEX minus cost on DEX minus gas.
    // Do NOT apply DEX fee again (already included in quote). Apply only CEX fee.
    let revenue_total = bid_price * token0_out * (1.0 - config.cex_fee_bps / 10_000.0);
    let cost_total = token1_in; // USDC spent already includes DEX LP fee
    let pnl = revenue_total - cost_total - gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let description = format!(
            "A: Buy {:.6} ETH on DEX → Sell on CEX @ ${:.2} | Earn ${:.2}",
            token0_out, bid_price, pnl
        );

        Some(ArbitrageOpportunity {
            direction: "A".to_string(),
            description,
            pnl,
        })
    } else {
        None
    }
}

/// Evaluate Direction B: buy on CEX -> sell on DEX
fn evaluate_direction_b(
    pool_state: &PoolState,
    book: &BookDepth,
    _dex_price: f64,
    config: &ArbitrageConfig,
    gas_cost_usdc: f64,
) -> Option<ArbitrageOpportunity> {
    let (ask_price, ask_qty_cex) = book.asks[0];
    let effective_ask_price = ask_price * (1.0 + config.cex_fee_bps / 10_000.0);
    let res = calculate_swap_with_library(
        pool_state,
        effective_ask_price,
        SwapDirection::Token1ToToken0,
        config.dex_fee_bps,
        ask_qty_cex,
    )
    .ok()?;

    let mut token0_in = res.amount_in; // ETH to sell on DEX
    let mut token1_out = res.amount_out; // USDC received from DEX

    // Clamp to CEX depth
    if token0_in > ask_qty_cex {
        let scale = ask_qty_cex / token0_in;
        token0_in = ask_qty_cex;
        token1_out *= scale;
    }

    if token0_in <= 1e-8 {
        return None;
    }

    // Calculate profit and loss: revenue on DEX minus cost on CEX minus gas
    let revenue_total = token1_out;
    let cost_total = effective_ask_price * token0_in;
    let pnl = revenue_total - cost_total - gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let description = format!(
            "B: Buy {:.6} ETH on CEX  → Sell on DEX @ ${:.2} | Earn ${:.2}",
            token0_in, ask_price, pnl
        );

        Some(ArbitrageOpportunity {
            direction: "B".to_string(),
            description,
            pnl,
        })
    } else {
        None
    }
}

/// Calculate gas cost in USDC
pub fn calculate_gas_cost_usdc(
    gas_gwei: f64,
    gas_units: f64,
    gas_multiplier: f64,
    dex_price: f64,
) -> f64 {
    gas_gwei * 1e-9 * gas_units * gas_multiplier * dex_price
}
