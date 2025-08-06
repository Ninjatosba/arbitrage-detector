//! Shared data structures used throughout the application.

use std::time::SystemTime;

/// Best bid/ask snapshot for a given trading pair.
#[derive(Debug, Clone, Copy)]
pub struct PricePoint {
    pub timestamp: SystemTime,
    pub bid: f64,
    pub ask: f64,
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
