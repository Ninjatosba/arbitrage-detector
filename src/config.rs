//! Configuration loader and application settings.

use crate::arbitrage::ArbitrageConfig;

/// Consolidated application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// RPC endpoint for the Ethereum-compatible node.
    pub rpc_url: String,
    /// WebSocket endpoint for the chosen CEX public feed.
    pub cex_ws_url: String,
    /// Trading pair symbol (e.g., "ETH/USDC").
    //pub pair: String,
    /// Pool address
    pub pool_address: String,
    /// Minimum PnL threshold to log opportunities
    pub min_pnl_usdc: f64,
    /// Gas configuration
    pub gas_config: GasConfig,
    /// Arbitrage config
    pub arbitrage_config: ArbitrageConfig,
}

impl AppConfig {
    /// Try to load configuration from environment variables.
    pub fn try_load() -> crate::errors::Result<Self> {
        let rpc_url = std::env::var("RPC_URL")?;
        let cex_ws_url = std::env::var("CEX_WS_URL")?;
        let pool_address = std::env::var("POOL_ADDRESS")?;
        let min_pnl_usdc: f64 = std::env::var("MIN_PNL_USDC")?.parse()?;
        let gas_units: f64 = std::env::var("GAS_UNITS")?.parse()?;
        let gas_multiplier: f64 = std::env::var("GAS_MULTIPLIER")?.parse()?;
        let dex_fee_bps: f64 = std::env::var("DEX_FEE_BPS")?.parse()?;
        let cex_fee_bps: f64 = std::env::var("CEX_FEE_BPS")?.parse()?;
        Ok(Self {
            rpc_url,
            cex_ws_url,
            pool_address,
            min_pnl_usdc,
            gas_config: GasConfig {
                gas_units,
                gas_multiplier,
            },
            arbitrage_config: ArbitrageConfig {
                min_pnl_usdc,
                dex_fee_bps,
                cex_fee_bps,
            },
        })
    }
}

/// Gas configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct GasConfig {
    pub gas_units: f64,
    pub gas_multiplier: f64,
}
