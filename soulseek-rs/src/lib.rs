//! The program itself. `main.rs` is a thin entry point over this crate, so
//! every module is compiled once and integration tests drive the same code the
//! binary runs.

pub mod api;
pub mod cli;
pub mod commands;
pub mod daemon;
pub(crate) mod directories;
pub mod models;
pub mod output;
pub mod persist;
pub(crate) mod port_mapping;
pub mod remote;
pub mod run;
pub(crate) mod ui;

/// The interactive window, reachable so an integration test can drive it
/// with key events, advance it a frame at a time and read the screen back.
/// Hidden from the documented API: it is a harness door, not a contract.
#[doc(hidden)]
pub use ui::MainTui;
