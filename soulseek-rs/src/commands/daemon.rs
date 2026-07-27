//! Controlling the service: `daemon token`, `daemon status`, `daemon stop`.
//!
//! Running the daemon itself lives in [`crate::daemon`]; these are the
//! one-shot commands that talk *to* one. None of them needs a Soulseek
//! account — the daemon already has one — so they are dispatched before
//! credentials are demanded.

use crate::daemon::proto::{DaemonStatus, Method};
use crate::output::{
    CliError, CliResult, DaemonStatusRecord, Out, TokenRecord,
};
use crate::remote::{Endpoint, RemoteSession};

/// Print the token a client on another machine needs.
///
/// Local clients never need this — the socket's permissions already answer the
/// question — so it exists for the moment you point a second machine at this
/// one.
pub fn token(out: &Out) -> CliResult {
    let path = crate::daemon::token_path().ok_or_else(|| {
        CliError::usage("no state directory: set SOULSEEK_STATE_DIR")
    })?;
    let token = crate::daemon::load_or_create_token(&path).map_err(|e| {
        CliError::usage(format!("cannot read {}: {e}", path.display()))
    })?;
    out.emit(&TokenRecord { token });
    Ok(())
}

/// Ask a running daemon what it is doing.
pub fn status(
    out: &Out,
    endpoint: Option<&Endpoint>,
    token: Option<String>,
) -> CliResult {
    let status: DaemonStatus = ask(endpoint, token, Method::DaemonStatus)?;
    out.emit(&DaemonStatusRecord {
        user: status.username,
        server: status.server,
        version: status.daemon_version,
        protocol: status.protocol,
        listening: status.listen_port.is_some(),
        listen_port: status.listen_port.unwrap_or(0),
        shared_folders: status.shared_folders,
        shared_files: status.shared_files,
        download_dir: status.download_dir,
        session_loss: status
            .session_loss
            .map(|loss| format!("{loss:?}").to_lowercase()),
        clients: status.clients,
        uptime_secs: status.uptime_secs,
    });
    Ok(())
}

/// Ask a running daemon to shut down.
pub fn stop(
    out: &Out,
    endpoint: Option<&Endpoint>,
    token: Option<String>,
) -> CliResult {
    let _: serde_json::Value = ask(endpoint, token, Method::DaemonStop)?;
    out.status("the daemon is stopping");
    Ok(())
}

/// One request to whichever daemon this run is pointed at.
fn ask<T: serde::de::DeserializeOwned>(
    endpoint: Option<&Endpoint>,
    token: Option<String>,
    method: Method,
) -> CliResult<T> {
    let endpoint = endpoint.ok_or_else(no_daemon)?;
    let token = token.or_else(crate::daemon::stored_token);
    let session = RemoteSession::connect(endpoint, token).map_err(|e| {
        CliError::connection(format!("cannot reach the daemon: {e}"))
    })?;
    session
        .ask_for(method)
        .map_err(|e| CliError::connection(e.to_string()))
}

fn no_daemon() -> CliError {
    CliError::connection(
        "no daemon is running; start one with `soulseek-rs daemon`, or point \
         at a remote one with --daemon",
    )
}
