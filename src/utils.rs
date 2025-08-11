//! Miscellaneous helper utilities.

use anyhow::Result;
use ethers::providers::{Http, Middleware, Provider};
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialize `tracing` subscriber with env-based filter.
///
/// If `RUST_LOG` is not set, defaults to `info` level.
pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}

/// Spawns a background task that periodically fetches EIP-1559 base fee and
/// updates a provided `tokio::sync::watch::Sender<f64>` with an average gas
/// price estimate in gwei. Caller decides the interval.
pub async fn spawn_gas_price_watcher(
    rpc_url: &str,
    tx: tokio::sync::watch::Sender<f64>,
    interval_secs: u64,
) -> Result<tokio::task::JoinHandle<()>> {
    let provider = Arc::new(Provider::<Http>::try_from(rpc_url)?);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            let mut gwei = 0.0f64;
            if let Ok(block) = provider.get_block(ethers::types::BlockNumber::Latest).await {
                if let Some(b) = block {
                    if let Some(base_fee) = b.base_fee_per_gas {
                        // Convert wei to gwei
                        let wei: u128 = base_fee.as_u128();
                        gwei = (wei as f64) / 1_000_000_000.0;
                    }
                }
            }
            let _ = tx.send(gwei);
        }
    });
    Ok(handle)
}
