use anyhow::Result;
use arbitrage_detector::{
    aggregator::spawn_arbitrage_evaluator,
    cex::spawn_cex_stream_watcher,
    config::AppConfig,
    dex::{Dex, init_pool_state_watcher},
    utils::{init_logging, spawn_gas_price_watcher},
};
use ethers::types::Address;
use std::str::FromStr;
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_logging();

    // Configuration
    let config = AppConfig::try_load()?;
    let gas_config = config.gas_config;
    let arbitrage_config = config.arbitrage_config;

    tracing::info!("[INIT] arbitrage-detector starting");

    // Shared state channels
    let (cex_tx, cex_rx) = watch::channel::<arbitrage_detector::models::BookDepth>(
        arbitrage_detector::models::BookDepth::default(),
    );

    // Initialize DEX
    let dex = Dex::new(&config.rpc_url, Address::from_str(&config.pool_address)?).await?;

    // Initialize pool state watcher
    let initial_pool_state = dex.get_pool_state(6, 18, None, None).await?;
    let (pool_tx, pool_rx) =
        watch::channel::<arbitrage_detector::dex::PoolState>(initial_pool_state);
    let _pool_handle = init_pool_state_watcher(&dex, pool_tx).await?;

    // Initialize gas price watcher
    let (gas_tx, gas_rx) = watch::channel::<f64>(0.0);
    let _gas_handle = spawn_gas_price_watcher(&config.rpc_url, gas_tx.clone(), 10).await?;
    tracing::info!("[INIT] gas watcher started (10s interval)");

    // Spawn producer tasks
    let cex_task = spawn_cex_stream_watcher("ethusdc", cex_tx).await?;

    // Spawn arbitrage evaluator
    let _evaluator_task =
        spawn_arbitrage_evaluator(cex_rx, pool_rx, gas_rx, gas_config, arbitrage_config).await;

    // Wait indefinitely for producer tasks (they never finish)
    let _ = futures::join!(cex_task);
    Ok(())
}
