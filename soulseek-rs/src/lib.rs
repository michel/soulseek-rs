//! Library surface of the TUI binary, exposed for integration tests.
//! The binary itself compiles these modules directly (see `main.rs`).

pub mod api;
pub mod cli;
pub mod commands;
pub mod daemon;
pub mod directories;
pub mod models;
pub mod output;
pub mod persist;
pub mod port_mapping;
pub mod remote;
