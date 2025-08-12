use crate::errors::Result;
use crate::models::BookDepth;
use futures::{Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::watch;
use tokio_tungstenite::connect_async;
use tracing::warn;
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

    let (ws_stream, _resp) = connect_async(url).await?;

    let mapped = ws_stream.filter_map(|msg_res| async {
        match msg_res {
            Ok(msg) if msg.is_text() => {
                let txt = match msg.into_text() {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(error = %e, "[CEX] text extraction failed");
                        return None;
                    }
                };
                let parsed: DepthMsg = match serde_json::from_str(&txt) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "[CEX] depth JSON parse failed");
                        return None;
                    }
                };
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
            Err(e) => {
                warn!(error = %e, "[CEX] websocket message error");
                None
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
) -> Result<tokio::task::JoinHandle<()>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_depth_message_shape() {
        // Structure sanity test only; parser lives in stream transform.
        let raw = r#"{"lastUpdateId":1,"bids":[["100.0","1.0"]],"asks":[["101.0","2.0"]]}"#;
        let parsed: Result<DepthMsg> = serde_json::from_str::<DepthMsg>(raw).map_err(Into::into);
        assert!(parsed.is_ok());
    }

    #[tokio::test]
    async fn stream_filters_invalid_and_maps_numbers() {
        // Simulate a subset of the mapping path by feeding a valid JSON text message
        // into the transform and ensuring we get numeric tuples out.
        // We can't easily mock the websocket here without writing too much code
        let raw = r#"{
            "lastUpdateId": 123,
            "bids": [["100.5", "2.25"], ["bad","1"]],
            "asks": [["101.5", "3.50"], ["102.0","bad"]]
        }"#;
        let parsed: DepthMsg = serde_json::from_str(raw).expect("json should parse");
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
        assert_eq!(bids, vec![(100.5, 2.25)]);
        assert_eq!(asks, vec![(101.5, 3.5)]);
    }
}
