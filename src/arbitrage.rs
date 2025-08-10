//! Arbitrage detection logic.

use crate::dex::{PoolState, solve_from_cex_bid, solve_token0_in_for_target_avg_price};
use crate::models::{BookDepth, Opportunity, PricePoint, TradeQuote};
use bigdecimal::BigDecimal;
use std::str::FromStr;

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
    let res = solve_from_cex_bid(pool_state, bid_price, bid_qty);

    let token1_in = res
        .amount_token1_in
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.0);
    let token0_out = res
        .amount_token0_out
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.0);

    if token0_out <= 0.0 {
        return None;
    }

    let revenue_total = bid_price * token0_out * (1.0 - config.cex_fee_bps / 10_000.0);
    let cost_total = token1_in * (1.0 + config.dex_fee_bps / 10_000.0);
    let pnl = revenue_total - cost_total - config.gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let buy_price = res
            .realized_avg_price_t1_per_t0
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0);
        let description = format!(
            "A: Buy {:.6} ETH on DEX @ ${:.2} → Sell on CEX @ ${:.2} | Earn ${:.2}",
            token0_out, buy_price, bid_price, pnl
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
    let price_bd =
        BigDecimal::from_str(&ask_price.to_string()).unwrap_or_else(|_| BigDecimal::from(0u32));
    let qty_bd =
        BigDecimal::from_str(&ask_qty.to_string()).unwrap_or_else(|_| BigDecimal::from(0u32));

    let res = solve_token0_in_for_target_avg_price(pool_state, &price_bd, Some(&qty_bd), None);

    let token0_in = res
        .amount_token0_in
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.0);
    let token1_out = res
        .amount_token1_out
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.0);

    if token0_in <= 1e-8 {
        return None;
    }

    let cost_total = ask_price * token0_in * (1.0 + config.cex_fee_bps / 10_000.0);
    let revenue_total = token1_out * (1.0 - config.dex_fee_bps / 10_000.0);
    let _pnl = revenue_total - cost_total - config.gas_cost_usdc;

    // Calculate actual sell price from amounts (after fees)
    let actual_sell_price = (token1_out * (1.0 - config.dex_fee_bps / 10_000.0)) / token0_in;

    // Calculate real PnL: (sell_price - buy_price) * amount - fees - gas
    let price_diff = actual_sell_price - ask_price;
    let gross_profit = price_diff * token0_in;
    let cex_fee = ask_price * token0_in * (config.cex_fee_bps / 10_000.0);
    let dex_fee = token1_out * (config.dex_fee_bps / 10_000.0);
    let real_pnl = gross_profit - cex_fee - dex_fee - config.gas_cost_usdc;

    if real_pnl >= config.min_pnl_usdc {
        let description = format!(
            "B: Buy {:.8} ETH on CEX @ ${:.2} → Sell on DEX @ ${:.2} | Earn ${:.2}",
            token0_in, ask_price, actual_sell_price, real_pnl
        );

        Some(ArbitrageOpportunity {
            direction: "B".to_string(),
            description,
            pnl: real_pnl,
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

/// Load arbitrage configuration from environment variables
pub fn load_arbitrage_config(min_pnl_usdc: f64, pool_fee_bps: f64) -> ArbitrageConfig {
    let dex_fee_bps = if std::env::var("IGNORE_DEX_FEE").unwrap_or_else(|_| "0".into()) == "1" {
        0.0
    } else {
        pool_fee_bps
    };

    let cex_fee_bps = std::env::var("CEX_FEE_BPS")
        .unwrap_or_else(|_| "10".into())
        .parse()
        .unwrap_or(10.0);

    ArbitrageConfig {
        min_pnl_usdc,
        dex_fee_bps,
        cex_fee_bps,
        gas_cost_usdc: 0.0, // Will be set later
    }
}

/// Evaluate whether an arbitrage opportunity exists (stub).
///
/// Returns `Some(Opportunity)` if profitable given the inputs, `None` otherwise.
pub fn detect(_cex: &PricePoint, _dex: &TradeQuote, _gas_cost: f64) -> Option<Opportunity> {
    todo!("Implement arbitrage math with fees and gas");
}
