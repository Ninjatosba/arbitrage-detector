//! CEX (Centralized Exchange) integration.

pub mod binance;

pub use binance::{connect_and_stream, spawn_cex_stream_watcher};
