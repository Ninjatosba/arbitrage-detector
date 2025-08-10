//! Arbitrage detection logic.

use crate::dex::{PoolState, SwapDirection, calculate_swap};
use crate::models::BookDepth;

/// Configuration for arbitrage calculations
#[derive(Debug, Clone)]
pub struct ArbitrageConfig {
    pub min_pnl_usdc: f64,
    pub dex_fee_bps: f64,
    pub cex_fee_bps: f64,
    pub gas_cost_usdc: f64,
}

/// Result of arbitrage opportunity evaluation
#[derive(Debug, Clone)]
pub struct ArbitrageOpportunity {
    pub direction: String,
    pub description: String,
    pub pnl: f64,
}

/// Evaluate arbitrage opportunities in both directions
pub fn evaluate_opportunities(
    pool_state: &PoolState,
    book: &BookDepth,
    dex_price: f64,
    config: &ArbitrageConfig,
) -> Vec<ArbitrageOpportunity> {
    let mut opportunities = Vec::new();

    if book.bids.is_empty() || book.asks.is_empty() {
        return opportunities;
    }

    // Direction A: buy on DEX -> sell on CEX (use CEX bid)
    if let Some(opp) = evaluate_direction_a(pool_state, book, dex_price, config) {
        opportunities.push(opp);
    }

    // Direction B: buy on CEX -> sell on DEX (use CEX ask)
    if let Some(opp) = evaluate_direction_b(pool_state, book, dex_price, config) {
        opportunities.push(opp);
    }

    opportunities
}

/// Evaluate Direction A: buy on DEX -> sell on CEX
fn evaluate_direction_a(
    pool_state: &PoolState,
    book: &BookDepth,
    _dex_price: f64,
    config: &ArbitrageConfig,
) -> Option<ArbitrageOpportunity> {
    let (bid_price, bid_qty) = book.bids[0];

    // Calculate how much USDC we need to spend to get ETH on DEX
    let max_usdc_input = bid_price * bid_qty; // Approximate cap
    let res = calculate_swap(
        pool_state,
        max_usdc_input,
        SwapDirection::Token1ToToken0, // USDC → ETH
        config.dex_fee_bps,
        bid_qty, // Max ETH we can sell on CEX
    );

    let token1_in = res.amount_in; // USDC spent
    let token0_out = res.amount_out; // ETH received

    if token0_out <= 0.0 {
        return None;
    }

    // Calculate PnL: (CEX sell price - DEX buy price) * amount - fees - gas
    let revenue_total = bid_price * token0_out * (1.0 - config.cex_fee_bps / 10_000.0);
    let cost_total = token1_in * (1.0 + config.dex_fee_bps / 10_000.0);
    let pnl = revenue_total - cost_total - config.gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let description = format!(
            "A: Buy {:.6} ETH on DEX @ ${:.2} → Sell on CEX @ ${:.2} | Earn ${:.2}",
            token0_out, res.execution_price, bid_price, pnl
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
) -> Option<ArbitrageOpportunity> {
    let (ask_price, ask_qty) = book.asks[0];

    // Calculate how much ETH we can sell on DEX for the CEX ask price
    let res = calculate_swap(
        pool_state,
        ask_qty,                       // ETH amount to sell
        SwapDirection::Token0ToToken1, // ETH → USDC
        config.dex_fee_bps,
        ask_price * ask_qty, // Max USDC we expect
    );

    let token0_in = res.amount_in; // ETH sold
    let token1_out = res.amount_out; // USDC received

    if token0_in <= 1e-8 {
        return None;
    }

    // Calculate PnL: (DEX sell price - CEX buy price) * amount - fees - gas
    let cost_total = ask_price * token0_in * (1.0 + config.cex_fee_bps / 10_000.0);
    let revenue_total = token1_out * (1.0 - config.dex_fee_bps / 10_000.0);
    let pnl = revenue_total - cost_total - config.gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let description = format!(
            "B: Buy {:.8} ETH on CEX @ ${:.2} → Sell on DEX @ ${:.2} | Earn ${:.2}",
            token0_in, ask_price, res.execution_price, pnl
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
