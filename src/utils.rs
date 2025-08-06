//! Miscellaneous helper utilities.

use tracing_subscriber::{fmt, EnvFilter};

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
