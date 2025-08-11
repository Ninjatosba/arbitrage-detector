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
