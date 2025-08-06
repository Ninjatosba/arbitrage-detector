use anyhow::Result;
use arbitrage_detector::{cex, dex::Dex, models::BookDepth, utils};
use ethers::types::Address;
use futures::StreamExt;
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

    // Amount used for VWAP calculations on the CEX side
    let cex_trade_size: f64 = std::env::var("TRADE_SIZE_ETH")
        .unwrap_or_else(|_| "1.0".into())
        .parse()
        .unwrap_or(1.0);

    // Shared state channels
    let (dex_tx, mut dex_rx) = watch::channel::<f64>(0.0);
    let (cex_tx, mut cex_rx) = watch::channel::<BookDepth>(BookDepth::default());

    // DEX producer ---------------------------------------------------------
    let dex = Dex::new(&rpc_url, pool_addr).await?;
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

    // Aggregator (log every 1 second) -------------------------------------
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            let dex_price = *dex_rx.borrow();
            let book = cex_rx.borrow().clone();
            if dex_price == 0.0 || book.bids.is_empty() || book.asks.is_empty() {
                continue;
            }
            let (best_bid, _) = book.bids[0];
            let (best_ask, _) = book.asks[0];
            let vwap_sell = book.vwap_sell_eth(cex_trade_size);
            let vwap_buy = book.vwap_buy_eth(cex_trade_size);

            tracing::info!(
                dex_price,
                best_bid,
                best_ask,
                vwap_sell,
                vwap_buy,
                "[SNAPSHOT] dex vs cex"
            );
        }
    });

    // Wait indefinitely for producer tasks (they never finish)
    let _ = futures::join!(dex_task, cex_task);
    Ok(())
}
