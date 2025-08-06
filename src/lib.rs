//! Core library for the arbitrage-detector project.
//!
//! This crate only defines the module boundaries and exposes
//! public stubs so that the binary (`main.rs`) can evolve
//! incrementally without compilation errors.

pub mod arbitrage;
pub mod cex;
pub mod cli;
pub mod config;
pub mod dex;
pub mod models;
pub mod utils;
