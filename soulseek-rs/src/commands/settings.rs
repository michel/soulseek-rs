//! Managing what this client shares and what it remembers: `shares` and
//! `config`.
//!
//! Both write to the same `config.toml` the TUI uses, so a change made here is
//! what the next run — interactive or scripted — starts from.

use super::{Ctx, Session};
use crate::api::SessionApi;
use crate::directories;
use crate::output::{
    CliError, CliResult, ConfigRecord, Out, ShareRecord, ShareStatusRecord,
};
use crate::persist::config::FileConfig;
use crate::remote::{Endpoint, RemoteSession};
use std::path::PathBuf;

/// Where a `shares`/`config` change is written, and what it starts from.
pub struct Store {
    pub path: Option<PathBuf>,
    pub config: FileConfig,
}

impl Store {
    pub fn save(&self, config: &FileConfig) -> CliResult {
        let Some(path) = &self.path else {
            return Err(CliError::usage(
                "no config file to write to (running with --no-config)",
            ));
        };
        config
            .save(path)
            .map_err(|e| CliError::usage(format!("cannot save config: {e}")))
    }
}

pub fn shares_list(out: &Out, store: &Store, resolved: &[String]) -> CliResult {
    let directories = configured_shares(store, resolved);
    if directories.is_empty() {
        return Err(CliError::no_results("no folders are shared"));
    }
    for directory in directories {
        out.emit(&ShareRecord {
            usable: directories::resolve_shared_directory(Some(&directory))
                .is_ok_and(|resolved| resolved.is_some()),
            directory,
        });
    }
    Ok(())
}

/// Where a running daemon can be reached, so a share change takes effect now
/// rather than at its next restart.
pub struct LiveDaemon {
    pub endpoint: Option<Endpoint>,
    pub token: Option<String>,
}

/// Tell a running daemon about a change it should apply now.
///
/// Locally a config change means "from the next run on", because there is no
/// session yet. A daemon *is* that next run and it is already going, so
/// leaving it on its startup settings would make the command look like it
/// had done nothing — the folder configured, and the network none the wiser.
fn push_to_daemon(
    out: &Out,
    daemon: &LiveDaemon,
    applied: &str,
    kept: &str,
    apply: impl FnOnce(&RemoteSession) -> soulseek_rs::Result<()>,
) {
    let Some(endpoint) = daemon.endpoint.as_ref() else {
        return;
    };
    let token = daemon.token.clone().or_else(crate::daemon::stored_token);
    let outcome = RemoteSession::connect(endpoint, token)
        .and_then(|session| apply(&session));
    match outcome {
        Ok(()) => out.status(applied),
        // Not fatal: the config file is written either way, so the change
        // still takes effect when the daemon next starts.
        Err(e) => out.warn(&format!(
            "the daemon kept {kept} ({e}); restart it to pick this up"
        )),
    }
}

fn push_shares(out: &Out, daemon: &LiveDaemon, directories: Vec<String>) {
    push_to_daemon(
        out,
        daemon,
        "the running daemon is re-indexing",
        "its own shares",
        move |session| session.set_shared_directories(directories),
    );
}

pub fn shares_add(
    out: &Out,
    store: &Store,
    directory: &str,
    daemon: &LiveDaemon,
) -> CliResult {
    // Refuse a folder that is not there: sharing it would silently index
    // nothing, and the mistake would only surface as "no one can find my
    // files" much later.
    match directories::resolve_shared_directory(Some(directory)) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(CliError::usage("no folder given")),
        Err(e) => return Err(CliError::usage(e)),
    }

    let mut updated = store.config.clone();
    let mut dirs = share_list(&store.config);
    if dirs.iter().any(|existing| existing == directory) {
        out.status(&format!("{directory} is already shared"));
        return Ok(());
    }
    dirs.push(directory.to_string());
    updated.shared_dirs = Some(dirs.clone());
    updated.shared_dir = None;
    store.save(&updated)?;
    out.status(&format!("sharing {directory}"));
    push_shares(out, daemon, dirs);
    Ok(())
}

pub fn shares_remove(
    out: &Out,
    store: &Store,
    directory: &str,
    daemon: &LiveDaemon,
) -> CliResult {
    let mut dirs = share_list(&store.config);
    let before = dirs.len();
    dirs.retain(|existing| existing != directory);
    if dirs.len() == before {
        return Err(CliError::no_results(format!(
            "{directory} is not in the shared folders"
        )));
    }

    let mut updated = store.config.clone();
    updated.shared_dirs = Some(dirs.clone());
    updated.shared_dir = None;
    store.save(&updated)?;
    out.status(&format!("no longer sharing {directory}"));
    push_shares(out, daemon, dirs);
    Ok(())
}

/// Connect with the configured shares so the library scans them, then report
/// what the network will actually see.
pub fn shares_status(ctx: &Ctx) -> CliResult {
    let session = Session::open(ctx)?;
    let (folders, files) = session.client.shared_counts();
    ctx.out.emit(&ShareStatusRecord {
        folders,
        files,
        // What the session shares, which locally is what this run configured
        // and against a daemon is what the daemon configured.
        directories: session.client.shared_directories(),
    });
    Ok(())
}

/// Re-scan the configured folders in a live session, so files added since the
/// last run become visible without waiting for the next one.
pub fn shares_reindex(ctx: &Ctx) -> CliResult {
    let session = Session::open(ctx)?;
    ctx.out.status("re-indexing the shared folders…");
    // Re-scan what the session already shares rather than imposing this run's
    // configuration on it: against a daemon that would silently replace the
    // daemon's shares with the client machine's.
    let directories = session.client.shared_directories();
    session
        .client
        .set_shared_directories(directories.clone())
        .map_err(|e| CliError::usage(format!("cannot re-index: {e}")))?;

    let (folders, files) = session.client.shared_counts();
    ctx.out.emit(&ShareStatusRecord {
        folders,
        files,
        directories,
    });
    Ok(())
}

/// The folders the config file asks for, falling back to what this run
/// resolved (which includes `--shared-dir` and the download-folder default).
fn configured_shares(store: &Store, resolved: &[String]) -> Vec<String> {
    let configured = share_list(&store.config);
    if configured.is_empty() {
        return resolved.to_vec();
    }
    configured
}

/// Both config spellings, flattened: `shared_dir` is the single-folder form.
fn share_list(config: &FileConfig) -> Vec<String> {
    let mut dirs = config.shared_dirs.clone().unwrap_or_default();
    if let Some(single) = &config.shared_dir
        && !single.trim().is_empty()
        && !dirs.iter().any(|existing| existing == single)
    {
        dirs.insert(0, single.clone());
    }
    dirs
}

pub fn config_path(out: &Out, store: &Store) -> CliResult {
    let Some(path) = &store.path else {
        return Err(CliError::no_results(
            "no config file in use (running with --no-config)",
        ));
    };
    out.emit(&ConfigRecord {
        key: "config".to_string(),
        value: Some(path.display().to_string()),
    });
    Ok(())
}

pub fn config_list(out: &Out, store: &Store) {
    for key in FileConfig::KEYS {
        out.emit(&ConfigRecord {
            key: (*key).to_string(),
            value: store.config.get(key),
        });
    }
}

pub fn config_get(out: &Out, store: &Store, key: &str) -> CliResult {
    let value = store.config.get(key).ok_or_else(|| unknown_or_unset(key))?;
    out.emit(&ConfigRecord {
        key: key.to_string(),
        value: Some(value),
    });
    Ok(())
}

pub fn config_set(
    out: &Out,
    store: &Store,
    key: &str,
    value: &str,
    daemon: impl FnOnce() -> LiveDaemon,
) -> CliResult {
    let mut updated = store.config.clone();
    updated.set(key, value).map_err(CliError::usage)?;
    store.save(&updated)?;
    let current = updated.get(key);
    out.emit(&ConfigRecord {
        key: key.to_string(),
        value: current.clone(),
    });
    // A running daemon holds the live settings, so for the keys it owns a
    // config write alone would look like it did nothing until its next
    // restart — the same trap `shares add` closes by pushing to the session.
    // `daemon` is resolved lazily: finding one means connecting to a socket,
    // which the other keys should not pay for.
    match key {
        "download_dir" => {
            if let Some(directory) = current {
                push_to_daemon(
                    out,
                    &daemon(),
                    "the running daemon will download there now",
                    "its own download folder",
                    move |session| session.set_download_directory(directory),
                );
            }
        }
        "shared_dirs" => push_shares(out, &daemon(), share_list(&updated)),
        _ => {}
    }
    Ok(())
}

fn unknown_or_unset(key: &str) -> CliError {
    if FileConfig::KEYS.contains(&key) {
        return CliError::no_results(format!("{key} is not set"));
    }
    CliError::usage(format!(
        "unknown setting '{key}' — try one of: {}",
        FileConfig::KEYS.join(", ")
    ))
}
