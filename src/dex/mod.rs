//! DEX integration for Uniswap V3 pools.

pub mod calc;
pub mod state;
pub mod client;

pub use calc::calculate_swap_with_library;
pub use state::PoolState;
pub use client::{Dex, init_pool_state_watcher};
