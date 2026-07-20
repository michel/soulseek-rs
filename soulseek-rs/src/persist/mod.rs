//! Persistence for the TUI binary: config file, secrets, and state.
//!
//! Everything here is deliberately outside `soulseek-rs-lib` (which stays
//! zero-dependency); the lib keeps receiving plain values via
//! `ClientSettings`.

pub mod config;
pub mod paths;
pub mod secret;
