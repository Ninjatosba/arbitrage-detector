//! Arbitrage detection logic.

use crate::models::{Opportunity, PricePoint, TradeDirection, TradeQuote};

/// Evaluate whether an arbitrage opportunity exists (stub).
///
/// Returns `Some(Opportunity)` if profitable given the inputs, `None` otherwise.
pub fn detect(_cex: &PricePoint, _dex: &TradeQuote, _gas_cost: f64) -> Option<Opportunity> {
    todo!("Implement arbitrage math with fees and gas");
}
