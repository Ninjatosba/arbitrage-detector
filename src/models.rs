//! Shared data structures used throughout the application.

use std::time::SystemTime;

/// Best bid/ask snapshot for a given trading pair.
#[derive(Debug, Clone, Copy)]
pub struct PricePoint {
    pub timestamp: SystemTime,
    pub bid: f64,
    pub ask: f64,
}

/// Extended ticker with available quantities (depth at top).
#[derive(Debug, Clone, Copy)]
pub struct BookTop {
    pub timestamp: SystemTime,
    pub bid: f64,
    pub bid_qty: f64,
    pub ask: f64,
    pub ask_qty: f64,
}

/// Depth snapshot (top N levels per side).
#[derive(Debug, Clone)]
pub struct BookDepth {
    pub timestamp: u64,
    /// (price, qty) pairs best → worst
    pub bids: Vec<(f64, f64)>,
    pub asks: Vec<(f64, f64)>,
}

impl Default for BookDepth {
    fn default() -> Self {
        Self {
            timestamp: 0,
            bids: Vec::new(),
            asks: Vec::new(),
        }
    }
}

impl BookDepth {
    /// Volume-weighted average price when SELLING `amount` ETH (hitting bids).
    /// Returns None if depth is insufficient.
    pub fn vwap_sell_eth(&self, mut amount: f64) -> Option<f64> {
        let mut value = 0.0;
        let original = amount;
        for (price, qty) in &self.bids {
            let take = qty.min(amount);
            value += take * price;
            amount -= take;
            if amount <= 0.0 {
                return Some(value / original);
            }
        }
        None // not enough depth
    }

    /// VWAP when BUYING `amount` ETH (hitting asks)
    pub fn vwap_buy_eth(&self, mut amount: f64) -> Option<f64> {
        let mut value = 0.0;
        let original = amount;
        for (price, qty) in &self.asks {
            let take = qty.min(amount);
            value += take * price;
            amount -= take;
            if amount <= 0.0 {
                return Some(value / original);
            }
        }
        None
    }
}

/// Detailed quote for swapping a fixed input amount on a DEX.
#[derive(Debug, Clone, Copy)]
pub struct TradeQuote {
    pub amount_in: f64,
    pub amount_out: f64,
    /// Liquidity‐provider fee in basis points.
    pub fee_bps: u32,
    /// Slippage estimate incurred by the trade in basis points.
    pub slippage_bps: u32,
}

/// Direction of the arbitrage leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDirection {
    BuyOnDexSellOnCex,
    BuyOnCexSellOnDex,
}

/// Calculated arbitrage opportunity including cost and profit numbers.
#[derive(Debug, Clone, Copy)]
pub struct Opportunity {
    pub direction: TradeDirection,
    pub gross_pnl: f64,
    pub net_pnl: f64,
}
