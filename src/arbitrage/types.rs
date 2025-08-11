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
