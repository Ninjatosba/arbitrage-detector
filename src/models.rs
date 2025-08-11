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

#[derive(Debug, Clone)]
pub struct SwapResult {
    pub amount_in: f64,
    pub amount_out: f64,
    pub hit_boundary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapDirection {
    /// token0 (USDC) in  → token1 (WETH) out → price UP  → √P decreases
    /// When CEX price > DEX price, buy ETH on DEX (USDC→ETH) to profit
    Token0ToToken1,
    /// token1 (WETH) in → token0 (USDC) out → price DOWN → √P increases
    /// When CEX price < DEX price, sell ETH on DEX (ETH→USDC) to profit
    Token1ToToken0,
}
