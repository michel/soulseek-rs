//! Scaffolding both CLI end-to-end binaries need: the environment they scrub,
//! the soulfind they run against, and a port nobody else holds.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Every environment variable the binary reads, cleared for each run so a
/// developer's shell (or a stray `.env`) cannot change what a test observes —
/// and so no run finds a real daemon.
pub const CLI_ENV_VARS: [&str; 15] = [
    "SOULSEEK_CONFIG_DIR",
    "SOULSEEK_STATE_DIR",
    "SOULSEEK_DAEMON",
    "SOULSEEK_DAEMON_TOKEN",
    "SOULSEEK_USERNAME",
    "SOULSEEK_PASSWORD",
    "SOULSEEK_PASSWORD_CMD",
    "SOULSEEK_SERVER",
    "SOULSEEK_NO_LISTENER",
    "SOULSEEK_LISTENER_PORT",
    "SOULSEEK_DOWNLOAD_DIR",
    "SOULSEEK_SHARED_DIR",
    "SOULSEEK_MAX_CONCURRENT_DOWNLOADS",
    "SOULSEEK_SEARCH_TIMEOUT",
    "SOULSEEK_CONFIG",
];

pub fn soulfind_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SOULFIND_BIN") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|dir| dir.join("soulfind/bin/soulfind"))
        .find(|candidate| candidate.exists())
}

pub fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()
        .map(|addr| addr.port())
}
