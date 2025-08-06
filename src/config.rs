//! Configuration loader and application settings.

/// Consolidated application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// RPC endpoint for the Ethereum-compatible node.
    pub rpc_url: String,
    /// WebSocket endpoint for the chosen CEX public feed.
    pub cex_ws_url: String,
    /// Trading pair symbol (e.g., "ETH/USDC").
    pub pair: String,
}

impl AppConfig {
    /// Load configuration from environment variables and CLI flags (placeholder).
    pub fn load() -> Self {
        todo!("Implement env + CLI loading logic");
    }
}
