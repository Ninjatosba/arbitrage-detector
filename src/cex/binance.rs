use crate::models::BookDepth;
use anyhow::{Context, Result};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::watch;
use tokio_tungstenite::connect_async;
use url::Url;

const BINANCE_WS_ENDPOINT: &str = "wss://stream.binance.com:9443/ws";

#[derive(Debug, Deserialize)]
struct DepthMsg {
    #[serde(rename = "lastUpdateId")]
    _last_update_id: u64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

/// Returns an asynchronous stream of `BookDepth`s for the given Binance symbol, e.g. "ethusdt".
pub async fn connect_and_stream(symbol: &str) -> Result<impl Stream<Item = BookDepth>> {
    let stream_path = format!("{}@depth20@100ms", symbol.to_lowercase());
    let url = Url::parse(&format!("{}/{}", BINANCE_WS_ENDPOINT, stream_path))?;

    let (ws_stream, _resp) = connect_async(url)
        .await
        .context("WebSocket connect failed")?;

    let mapped = ws_stream.filter_map(|msg_res| async {
        match msg_res {
            Ok(msg) if msg.is_text() => {
                let txt = msg.into_text().ok()?;
                let parsed: DepthMsg = serde_json::from_str(&txt).ok()?;
                let bids: Vec<(f64, f64)> = parsed
                    .bids
                    .iter()
                    .filter_map(|lvl| Some((lvl[0].parse().ok()?, lvl[1].parse().ok()?)))
                    .collect();
                let asks: Vec<(f64, f64)> = parsed
                    .asks
                    .iter()
                    .filter_map(|lvl| Some((lvl[0].parse().ok()?, lvl[1].parse().ok()?)))
                    .collect();
                if bids.is_empty() || asks.is_empty() {
                    return None;
                }
                Some(BookDepth {
                    timestamp: parsed._last_update_id,
                    bids,
                    asks,
                })
            }
            _ => None,
        }
    });
    Ok(mapped)
}

/// Spawn CEX stream watcher task
pub async fn spawn_cex_stream_watcher(
    symbol: &str,
    cex_tx: watch::Sender<BookDepth>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let symbol = symbol.to_string();

    let handle = tokio::spawn(async move {
        if let Ok(stream) = connect_and_stream(&symbol).await {
            futures::pin_mut!(stream);
            while let Some(book) = stream.next().await {
                let _ = cex_tx.send(book.clone());
            }
        }
    });

    Ok(handle)
}
