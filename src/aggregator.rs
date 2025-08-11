//! Aggregator logic for evaluating arbitrage opportunities.

use crate::{
    arbitrage::{ArbitrageConfig, calculate_gas_cost_usdc, evaluate_opportunities},
    dex::PoolState,
    models::BookDepth,
    utils::GasConfig,
};
use tokio::sync::watch;
use tracing;

/// Spawn the main arbitrage evaluation loop
pub async fn spawn_arbitrage_evaluator(
    cex_rx: watch::Receiver<BookDepth>,
    pool_rx: watch::Receiver<PoolState>,
    gas_rx: watch::Receiver<f64>,
    gas_config: GasConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut ticks: u64 = 0;

        loop {
            ticker.tick().await;
            ticks += 1;

            let book = cex_rx.borrow().clone();
            let pool_state = pool_rx.borrow().clone();
            let gas_gwei = *gas_rx.borrow();

            if book.bids.is_empty() || book.asks.is_empty() {
                if ticks % 5 == 0 {
                    tracing::info!("[HEARTBEAT] waiting for streams (dex or cex not ready)");
                }
                continue;
            }

            let dex_price = pool_state.price_usdc_per_eth;

            // Calculate gas cost
            let gas_cost_usdc = calculate_gas_cost_usdc(
                gas_gwei,
                gas_config.gas_units,
                gas_config.gas_multiplier,
                pool_state.price_usdc_per_eth,
            );

            // Load arbitrage configuration
            let config = ArbitrageConfig {
                min_pnl_usdc: 0.0,
                dex_fee_bps: 30.0,
                cex_fee_bps: 10.0,
                gas_cost_usdc,
            };

            // Evaluate opportunities
            let opportunities = evaluate_opportunities(&pool_state, &book, dex_price, &config);

            if !opportunities.is_empty() {
                let opportunity_logs: Vec<String> = opportunities
                    .iter()
                    .map(|opp| opp.description.clone())
                    .collect();
                tracing::info!(opps = ?opportunity_logs, "[OPP] opportunities found");
            } else if ticks % 5 == 0 {
                let (bid_price, _bid_qty) = book.bids[0];
                let (ask_price, _ask_qty) = book.asks[0];
                tracing::info!(
                    dex_price,
                    bid_price,
                    ask_price,
                    gas_gwei,
                    config.dex_fee_bps,
                    config.cex_fee_bps,
                    config.gas_cost_usdc,
                    "[HEARTBEAT] no opps above threshold"
                );
            }
        }
    })
}
