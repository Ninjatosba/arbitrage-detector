use anyhow::Result;
use arbitrage_detector::{
    cex,
    dex::{Dex, PoolState, solve_from_cex_bid, solve_token0_in_for_target_avg_price},
    models::BookDepth,
    utils,
};
use bigdecimal::BigDecimal;
use ethers::types::Address;
use futures::StreamExt;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    utils::init_logging();

    // Configuration
    let rpc_url =
        std::env::var("RPC_URL").expect("Set RPC_URL env var to your Ethereum node HTTP endpoint");
    let default_pool = "0x8ad599c3a0ff1de082011efddc58f1908eb6e6d8"; // Uniswap V3 ETH/USDC 0.3%
    let pool_addr_raw = std::env::var("POOL_ADDRESS").unwrap_or_else(|_| default_pool.into());
    let pool_addr: Address = pool_addr_raw.parse()?;

    // Minimum PnL threshold to log opportunities
    let min_pnl_usdc: f64 = std::env::var("MIN_PNL_USDC")
        .unwrap_or_else(|_| "0.5".into())
        .parse()
        .unwrap_or(0.5);

    tracing::info!(min_pnl_usdc, "[INIT] arbitrage-detector starting");

    // Shared state channels
    let (dex_tx, mut dex_rx) = watch::channel::<f64>(0.0);
    let (cex_tx, mut cex_rx) = watch::channel::<BookDepth>(BookDepth::default());

    // DEX producer ---------------------------------------------------------
    let dex = Dex::new(&rpc_url, pool_addr).await?;

    // Background PoolState watcher (multi-tick state)
    let (pool_tx, mut pool_rx) = {
        // initial state fetch
        let initial = dex
            .get_pool_state(
                18,   // token0 decimals (WETH)
                6,    // token1 decimals (USDC)
                None, // optional current tick lower sqrt bound
                None, // optional current tick upper sqrt bound
                12,   // max segments per side
            )
            .await?;
        let (tx, rx) = watch::channel::<PoolState>(initial);
        // spawn periodic updater (every 5s)
        let _handle = dex
            .spawn_pool_state_watcher(18, 6, 5, 12, tx.clone())
            .await?;
        (tx, rx)
    };

    // Background Gas watcher (base fee gwei)
    let (gas_tx, mut gas_rx) = watch::channel::<f64>(0.0);
    let _gas_handle = utils::spawn_gas_price_watcher(&rpc_url, gas_tx.clone(), 10).await?;
    tracing::info!("[INIT] gas watcher started (10s interval)");
    let dex_task = {
        let dex_clone = dex.clone();
        let dex_tx = dex_tx.clone();
        tokio::spawn(async move {
            loop {
                match dex_clone.fetch_price_usdc_per_eth().await {
                    Ok(price_bd) => {
                        let price_f64: f64 = price_bd.to_string().parse().unwrap_or(0.0);
                        let _ = dex_tx.send(price_f64);
                    }
                    Err(e) => {
                        tracing::warn!(?e, "DEX fetch error");
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    };

    // CEX producer ---------------------------------------------------------
    let cex_task = {
        let mut stream = cex::connect_and_stream("ethusdt").await?;
        let cex_tx = cex_tx.clone();
        tokio::spawn(async move {
            futures::pin_mut!(stream);
            while let Some(book) = stream.next().await {
                let _ = cex_tx.send(book.clone());
            }
        })
    };

    // Aggregator (evaluate every 1 second; only log if opportunity) --------
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        let mut ticks: u64 = 0;
        // Resolve pool fee once at start; could refresh periodically if needed
        let pool_fee_bps = {
            // best-effort; if fails fall back to 0.30%
            if let Ok(bps) = dex.get_pool_fee_bps().await {
                bps as f64
            } else {
                30.0
            }
        };
        // Gas config
        let gas_units: f64 = std::env::var("GAS_UNITS")
            .unwrap_or_else(|_| "350000".into())
            .parse()
            .unwrap_or(350000.0);
        let gas_multiplier: f64 = std::env::var("GAS_MULTIPLIER")
            .unwrap_or_else(|_| "1.2".into())
            .parse()
            .unwrap_or(1.2);
        tracing::info!(
            pool_fee_bps,
            gas_units,
            gas_multiplier,
            min_pnl_usdc,
            "[INIT] aggregator started"
        );
        loop {
            ticker.tick().await;
            ticks += 1;
            let dex_price = *dex_rx.borrow();
            let book = cex_rx.borrow().clone();
            let pool_state = pool_rx.borrow().clone();
            let gas_gwei = *gas_rx.borrow();
            if dex_price == 0.0 || book.bids.is_empty() || book.asks.is_empty() {
                if ticks % 5 == 0 {
                    tracing::info!("[HEARTBEAT] waiting for streams (dex or cex not ready)");
                }
                continue;
            }
            // Fees and cost constants (basis points)
            // Allow ignoring DEX fee via env for test runs
            let dex_fee_bps: f64 =
                if std::env::var("IGNORE_DEX_FEE").unwrap_or_else(|_| "0".into()) == "1" {
                    0.0
                } else {
                    pool_fee_bps
                };
            let cex_fee_bps: f64 = std::env::var("CEX_FEE_BPS")
                .unwrap_or_else(|_| "10".into())
                .parse()
                .unwrap_or(0.0);
            // Gas in USDC = gas_gwei * 1e-9 ETH/gas * gas_units * gas_multiplier * (USDC/ETH)
            let gas_cost_usdc: f64 = gas_gwei * 1e-9 * gas_units * gas_multiplier * dex_price;

            let mut opportunity_logs = Vec::new();

            // -------- Direction A: buy on DEX -> sell on CEX (use CEX bid) --------
            let (bid_price, bid_qty) = book.bids[0];
            // Use actual CEX bid price
            let test_bid_price = bid_price;
            let res_a =
                arbitrage_detector::dex::solve_from_cex_bid(&pool_state, test_bid_price, bid_qty);
            let token1_in = res_a
                .amount_token1_in
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.0);
            let token0_out = res_a
                .amount_token0_out
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.0);

            let revenue_total = bid_price * token0_out * (1.0 - cex_fee_bps / 10_000.0);
            let cost_total = token1_in * (1.0 + dex_fee_bps / 10_000.0);
            let pnl_total_a = revenue_total - cost_total - gas_cost_usdc;

            if token0_out > 0.0 && pnl_total_a >= min_pnl_usdc {
                let buy_price = res_a
                    .realized_avg_price_t1_per_t0
                    .to_string()
                    .parse::<f64>()
                    .unwrap_or(0.0);
                opportunity_logs.push(format!(
                    "A: Buy {:.6} ETH on DEX @ ${:.2} → Sell on CEX @ ${:.2} | Earn ${:.2}",
                    token0_out, buy_price, bid_price, pnl_total_a
                ));
            }

            // -------- Direction B: buy on CEX -> sell on DEX (use CEX ask) --------
            let (ask_price, ask_qty) = book.asks[0];
            // Use actual CEX ask price
            let test_ask_price = ask_price;
            let price_bd = BigDecimal::from_str(&test_ask_price.to_string())
                .unwrap_or_else(|_| BigDecimal::from(0u32));
            let qty_bd = BigDecimal::from_str(&ask_qty.to_string())
                .unwrap_or_else(|_| BigDecimal::from(0u32));
            // Prefer single-tick sizing to avoid noisy multi-tick extremes for now
            let res_b = arbitrage_detector::dex::solve_token0_in_for_target_avg_price(
                &pool_state,
                &price_bd,
                Some(&qty_bd),
                None,
            );
            let token0_in = res_b
                .amount_token0_in
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.0);
            let token1_out = res_b
                .amount_token1_out
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.0);

            let cost_total_b = ask_price * token0_in * (1.0 + cex_fee_bps / 10_000.0);
            let revenue_total_b = token1_out * (1.0 - dex_fee_bps / 10_000.0);
            let pnl_total_b = revenue_total_b - cost_total_b - gas_cost_usdc;

            // Calculate actual sell price from amounts (after fees)
            let actual_sell_price = if token0_in > 0.0 {
                (token1_out * (1.0 - dex_fee_bps / 10_000.0)) / token0_in
            } else {
                0.0
            };

            // Calculate real PnL: (sell_price - buy_price) * amount - fees - gas
            let real_pnl = if token0_in > 0.0 {
                let price_diff = actual_sell_price - ask_price;
                let gross_profit = price_diff * token0_in;
                let cex_fee = ask_price * token0_in * (cex_fee_bps / 10_000.0);
                let dex_fee = token1_out * (dex_fee_bps / 10_000.0);
                gross_profit - cex_fee - dex_fee - gas_cost_usdc
            } else {
                0.0
            };

            // Log any non-negative pnl that meets threshold
            if token0_in > 1e-8 && real_pnl >= min_pnl_usdc {
                opportunity_logs.push(format!(
                    "B: Buy {:.8} ETH on CEX @ ${:.2} → Sell on DEX @ ${:.2} | Earn ${:.2}",
                    token0_in, ask_price, actual_sell_price, real_pnl
                ));
            }
            if !opportunity_logs.is_empty() {
                tracing::info!(opps = ?opportunity_logs, "[OPP] opportunities found");
            } else if ticks % 5 == 0 {
                let (bid_price, _bid_qty) = book.bids[0];
                let (ask_price, _ask_qty) = book.asks[0];
                tracing::info!(
                    dex_price,
                    bid_price,
                    ask_price,
                    gas_gwei,
                    pool_fee_bps,
                    min_pnl_usdc,
                    "[HEARTBEAT] no opps above threshold"
                );
            }
        }
    });

    // Wait indefinitely for producer tasks (they never finish)
    let _ = futures::join!(dex_task, cex_task);
    Ok(())
}
