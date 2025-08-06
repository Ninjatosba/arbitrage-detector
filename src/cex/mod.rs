//! CEX WebSocket client.
//! 
//! Responsibilities:
//! • Maintain connection to a centralized exchange public feed.
//! • Keep the latest best bid / ask for a trading pair.
//! • Handle reconnection and backoff.

use crate::models::PricePoint;

/// Connect to the CEX WebSocket and stream `PricePoint` updates (stub).
/// Returns nothing for now; we will change the return type once dependencies are in place.
pub async fn connect_and_stream() -> Option<PricePoint> {
    todo!("Implement WebSocket client");
}
