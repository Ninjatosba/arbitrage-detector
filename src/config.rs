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
    /// Pool address
    pub pool_address: String,
    /// Minimum PnL threshold to log opportunities
    pub min_pnl_usdc: f64,
}

impl AppConfig {
    /// Load configuration from environment variables and CLI flags (placeholder).
    pub fn load() -> Self {
        let rpc_url = std::env::var("RPC_URL")
            .expect("Set RPC_URL env var to your Ethereum node HTTP endpoint");
        let cex_ws_url =
            std::env::var("CEX_WS_URL").expect("Set CEX_WS_URL env var to your CEX public feed");
        let pair = std::env::var("PAIR").expect("Set PAIR env var to your trading pair symbol");
        let pool_address =
            std::env::var("POOL_ADDRESS").expect("Set POOL_ADDRESS env var to your pool address");
        let min_pnl_usdc = std::env::var("MIN_PNL_USDC")
            .expect("Set MIN_PNL_USDC env var to your minimum PnL threshold")
            .parse()
            .expect("MIN_PNL_USDC must be a valid floating point number");
        Self {
            rpc_url,
            cex_ws_url,
            pair,
            pool_address,
            min_pnl_usdc,
        }
    }
}

/// Gas configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct GasConfig {
    pub gas_units: f64,
    pub gas_multiplier: f64,
}

/// Load gas configuration from environment variables
pub fn load_gas_config() -> GasConfig {
    let gas_units: f64 = std::env::var("GAS_UNITS")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .unwrap_or(350000.0);

    let gas_multiplier: f64 = std::env::var("GAS_MULTIPLIER")
        .unwrap_or_else(|_| "1.0".into())
        .parse()
        .unwrap_or(1.2);

    GasConfig {
        gas_units,
        gas_multiplier,
    }
}
