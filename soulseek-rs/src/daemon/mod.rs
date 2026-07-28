//! The resident service: one logged-in session, controlled over a socket.
//!
//! Where a one-shot command logs in, does its job and exits, the daemon logs in
//! once and stays. That is what makes cross-invocation work possible — a queue
//! that survives the command that filled it, a chat window that was open while
//! nobody was watching — and what lets several clients share one session
//! instead of fighting the server's one-login-per-account rule.
//!
//! Two transports, two answers to "who is this":
//!
//! * A Unix socket at `<state-dir>/daemon.sock`, mode 0600. The socket's own
//!   permissions are the authentication, the same bargain Docker and ssh-agent
//!   make, so a local client — an agent especially — needs no flags and no
//!   token.
//! * TCP, opt-in, always requiring the token in `<state-dir>/daemon.token`.

pub mod hub;
/// Generates `docs/openrpc.json` from the wire types and checks it is current.
/// Test-only: the published document is served at runtime by `include_str!`.
#[cfg(test)]
mod openrpc;
pub mod proto;
pub mod server;

use crate::api::SessionApi;
use crate::commands::{Ctx, Session};
use crate::output::{CliError, CliResult};
use crate::persist::state::{PersistedDownload, PersistedMessage, StateStore};
use hub::{Hub, PendingBrowses, PendingDownload};
use proto::ChatMessageDto;
use server::Daemon;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The control socket, where a local client looks without being told to.
pub const SOCKET_NAME: &str = "daemon.sock";
/// The shared secret for TCP clients — the cookie-file convention, so a local
/// client can pick it up without the operator handling it at all.
pub const TOKEN_NAME: &str = "daemon.token";

/// How often daemon-owned state is written back. Frequent enough that a kill
/// loses little, rare enough not to churn the disk on a busy queue.
const SAVE_INTERVAL: Duration = Duration::from_secs(5);

/// Open files the daemon asks the OS for at startup.
///
/// The client holds a socket per peer and will talk to a thousand of them
/// during a busy search, plus transfers, the share index, and control
/// connections. A shell hands its children a soft limit of 256 on macOS, which
/// a single search blows through — and a one-shot command never noticed
/// because it exited before it mattered. A resident process has to ask.
const WANTED_OPEN_FILES: u64 = 8192;

#[must_use]
pub fn socket_path() -> Option<PathBuf> {
    crate::persist::paths::state_dir().map(|dir| dir.join(SOCKET_NAME))
}

#[must_use]
pub fn token_path() -> Option<PathBuf> {
    crate::persist::paths::state_dir().map(|dir| dir.join(TOKEN_NAME))
}

/// The token a local client should present, when there is one to find.
#[must_use]
pub fn stored_token() -> Option<String> {
    let token = std::fs::read_to_string(token_path()?).ok()?;
    let token = token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// Read the token, or mint one on first start.
///
/// Written 0600 and never printed to stdout, so `daemon` output stays usable in
/// a pipe while the secret is still one `daemon token` away.
pub fn load_or_create_token(path: &Path) -> std::io::Result<String> {
    if let Some(existing) = std::fs::read_to_string(path)
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
    {
        return Ok(existing);
    }

    let token = mint_token();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{token}\n"))?;
    restrict(path)?;
    Ok(token)
}

/// 256 bits from the OS, hex-encoded. Long enough that the one-second penalty
/// on a wrong answer is the only rate limit the daemon needs.
fn mint_token() -> String {
    use std::fmt::Write;

    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS always has randomness");
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut token, byte| {
            let _ = write!(token, "{byte:02x}");
            token
        })
}

/// Make a secret readable by nobody else. A no-op where the platform has no
/// such notion, which is why TCP is opt-in rather than the default.
#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
const fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Whether a daemon is listening on the local socket right now.
///
/// This is what makes daemon mode seamless: a client that finds a live socket
/// uses it, and one that does not behaves exactly as it always has. A socket
/// file left by a crash answers `None`, so a stale file cannot strand a client.
#[must_use]
pub fn local_daemon() -> Option<PathBuf> {
    let path = socket_path()?;
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(&path)
            .ok()
            .map(|_| path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Run the daemon until it is asked to stop.
pub fn run(
    ctx: &Ctx,
    bind: Option<&str>,
    upload_slots: Option<u16>,
) -> CliResult {
    let state_dir = crate::persist::paths::state_dir().ok_or_else(|| {
        CliError::usage("no state directory: set SOULSEEK_STATE_DIR")
    })?;
    std::fs::create_dir_all(&state_dir).map_err(|e| {
        CliError::usage(format!(
            "cannot use state directory {}: {e}",
            state_dir.display()
        ))
    })?;

    let token = load_or_create_token(&state_dir.join(TOKEN_NAME))
        .map_err(|e| CliError::usage(format!("cannot read the token: {e}")))?;

    // Before anything opens a socket: running out of file descriptors mid
    // search leaves the client unable to accept peers *or* control
    // connections, and the only symptom is everything quietly failing.
    match raise_open_file_limit() {
        Ok(limit) if limit < WANTED_OPEN_FILES => ctx.out.warn(&format!(
            "only {limit} open files allowed; a busy search wants \
             {WANTED_OPEN_FILES}. Raise it with `ulimit -n {WANTED_OPEN_FILES}` \
             before starting the daemon."
        )),
        Ok(_) => {}
        Err(e) => ctx
            .out
            .warn(&format!("could not check the open-file limit: {e}")),
    }

    // Bind before logging in: a second daemon should fail in a millisecond,
    // not after a full login handshake against the server.
    let mut listeners = Vec::new();
    #[cfg(unix)]
    {
        let path = state_dir.join(SOCKET_NAME);
        listeners.push(server::bind_unix(&path).map_err(|e| {
            CliError::usage(format!("cannot listen on {}: {e}", path.display()))
        })?);
        ctx.out.status(&format!("listening on {}", path.display()));
    }
    if let Some(address) = bind {
        listeners.push(server::bind_tcp(address).map_err(|e| {
            CliError::connection(format!("cannot listen on {address}: {e}"))
        })?);
        ctx.out.status(&format!(
            "listening on {address} (remote clients need the token from \
             `soulseek-rs daemon token`)"
        ));
    }
    if listeners.is_empty() {
        return Err(CliError::usage(
            "nothing to listen on: pass --bind on a platform without Unix \
             sockets",
        ));
    }

    let session = Session::open(ctx)?;
    if let Some(slots) = upload_slots {
        session.client.set_upload_slots(usize::from(slots));
    }

    let store = StateStore::new(state_dir);
    let hub = Arc::new(Hub::new());
    let browses = Arc::new(PendingBrowses::default());
    let downloads = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));

    let rooms = restore(ctx, &store, session.client.as_ref(), &hub, &downloads);

    let daemon = Arc::new(Daemon {
        session: Arc::clone(&session.client),
        hub: Arc::clone(&hub),
        browses: Arc::clone(&browses),
        downloads: Arc::clone(&downloads),
        download_dir: Mutex::new(ctx.download_dir.clone()),
        server: ctx.settings.server_address.to_string(),
        token: token.clone(),
        started: Instant::now(),
        stop: Arc::clone(&stop),
        open: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        rooms: Mutex::new(rooms),
        searches: Mutex::new(std::collections::HashMap::new()),
    });

    let drainer = {
        let session = Arc::clone(&session.client);
        let hub = Arc::clone(&hub);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            hub::run(session, hub, browses, downloads, stop);
        })
    };

    for listener in listeners {
        let daemon = Arc::clone(&daemon);
        std::thread::spawn(move || listener.serve(daemon));
    }

    let (folders, files) = session.client.shared_counts();
    ctx.out.status(&format!(
        "logged in as {}, sharing {files} files in {folders} folders",
        session.client.username()
    ));
    ready(ctx, bind, &token);

    // The listeners run on their own threads; this one only has to notice a
    // `daemon.stop` and write state back on the way past.
    let mut last_save = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(hub::POLL);
        if last_save.elapsed() >= SAVE_INTERVAL {
            last_save = Instant::now();
            save(&store, session.client.as_ref(), &hub, &daemon);
        }
    }

    save(&store, session.client.as_ref(), &hub, &daemon);
    ctx.out.status("stopping");
    // The drainer watches the same flag; joining keeps a half-written state
    // file from racing the process exit.
    let _ = drainer.join();
    cleanup();
    Ok(())
}

/// Ask the OS for enough open files to run a busy session, and report what we
/// actually got.
///
/// Raising our own soft limit is what a service does: the limit a shell
/// inherits is sized for a shell. The hard limit is the ceiling, and asking
/// past it fails outright rather than clamping, so this asks for the smaller
/// of what we want and what we are allowed.
#[cfg(unix)]
fn raise_open_file_limit() -> std::io::Result<u64> {
    // SAFETY: every call takes a pointer to a `rlimit` this function owns, and
    // none of them retains it.
    unsafe {
        let mut limit = std::mem::zeroed::<libc::rlimit>();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if limit.rlim_cur >= WANTED_OPEN_FILES {
            return Ok(limit.rlim_cur);
        }

        let previous = limit.rlim_cur;
        let ceiling = if limit.rlim_max == libc::RLIM_INFINITY {
            WANTED_OPEN_FILES
        } else {
            limit.rlim_max.min(WANTED_OPEN_FILES)
        };

        // macOS rejects an unlimited *hard* limit for open files, so raising
        // the soft one while leaving `rlim_max` as it came back fails with
        // EINVAL. Naming both is what it will accept.
        for candidate in [
            libc::rlimit {
                rlim_cur: ceiling,
                rlim_max: limit.rlim_max,
            },
            libc::rlimit {
                rlim_cur: ceiling,
                rlim_max: ceiling,
            },
        ] {
            if libc::setrlimit(libc::RLIMIT_NOFILE, &raw const candidate) == 0 {
                return Ok(ceiling);
            }
        }
        Ok(previous)
    }
}

#[cfg(not(unix))]
const fn raise_open_file_limit() -> std::io::Result<u64> {
    Ok(WANTED_OPEN_FILES)
}

/// Say what is now possible, once, on stderr.
///
/// The token goes here rather than only into a file because the moment you
/// need it is the moment you bind a port, and hunting for `daemon token`
/// afterwards is a poor first experience. stdout stays empty either way, so
/// `daemon` still composes in a pipeline.
fn ready(ctx: &Ctx, bind: Option<&str>, token: &str) {
    const BANNER: &str = "
ready — leave this running.

  On this machine, nothing else to do. Other commands find
  it on their own and share this one login:

      soulseek-rs search 'aphex twin'
      soulseek-rs                       (the TUI attaches)
      soulseek-rs daemon status
      soulseek-rs daemon stop
";
    const LOCAL_ONLY: &str =
        "  To drive it from another machine, restart with --bind ADDR;
  `soulseek-rs daemon token` prints the secret it needs.
";

    ctx.out.status(BANNER);

    let Some(address) = bind else {
        ctx.out.status(LOCAL_ONLY);
        return;
    };

    ctx.out.status(&format!(
        "  From another machine, point a client at {address} with
  this token:

      {token}

      soulseek-rs config set daemon <this-host>:PORT
      soulseek-rs config set daemon_token <token above>

  That port has no encryption. Keep it off the open internet and \
         reach it over SSH instead.
"
    ));
}

/// Remove the socket file so the next start does not have to reason about
/// whether it is stale.
fn cleanup() {
    #[cfg(unix)]
    if let Some(path) = socket_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Bring back what the last run left: re-queue unfinished transfers, rejoin
/// rooms, and reload the conversation.
fn restore(
    ctx: &Ctx,
    store: &StateStore,
    session: &dyn SessionApi,
    hub: &Hub,
    downloads: &Mutex<Vec<PendingDownload>>,
) -> Vec<String> {
    hub.restore(
        store
            .load_messages()
            .into_iter()
            .map(|message| ChatMessageDto {
                peer: message.peer,
                outgoing: message.outgoing,
                text: message.text,
                at: message.at.timestamp(),
            })
            .collect(),
    );

    let rooms = store.load_rooms();
    for room in &rooms {
        if let Err(e) = session.join_room(room) {
            ctx.out.warn(&format!("cannot rejoin {room}: {e}"));
        }
    }

    let unfinished: Vec<PersistedDownload> = store
        .load_downloads()
        .into_iter()
        .filter(|download| !download.completed)
        .collect();
    if unfinished.is_empty() {
        return rooms;
    }
    ctx.out
        .status(&format!("resuming {} transfers", unfinished.len()));
    for download in unfinished {
        match session.download(
            download.filename.clone(),
            download.username.clone(),
            download.size,
            download.download_directory.clone(),
        ) {
            Ok((_, updates)) => {
                if let Ok(mut pending) = downloads.lock() {
                    pending.push(PendingDownload {
                        username: download.username,
                        filename: download.filename,
                        updates,
                    });
                }
            }
            Err(e) => ctx
                .out
                .warn(&format!("cannot resume {}: {e}", download.filename)),
        }
    }
    rooms
}

/// Write daemon-owned state back. Best-effort: a state file that cannot be
/// written is not a reason to drop a working session.
fn save(
    store: &StateStore,
    session: &dyn SessionApi,
    hub: &Hub,
    daemon: &Daemon,
) {
    let downloads: Vec<PersistedDownload> = session
        .get_all_downloads()
        .iter()
        .map(|download| PersistedDownload {
            username: download.username.clone(),
            filename: download.filename.clone(),
            size: download.size,
            download_directory: download.download_directory.clone(),
            completed: matches!(
                download.status,
                soulseek_rs::DownloadStatus::Completed
            ),
        })
        .collect();
    let _ = store.save_downloads(&downloads);

    let messages: Vec<PersistedMessage> = hub
        .history()
        .into_iter()
        .map(|message| PersistedMessage {
            peer: message.peer,
            outgoing: message.outgoing,
            text: message.text,
            at: chrono::DateTime::from_timestamp(message.at, 0)
                .unwrap_or_default()
                .into(),
        })
        .collect();
    let _ = store.save_messages(&messages);
    let _ = store.save_rooms(&daemon.rooms());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_minted_once_and_then_reused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(TOKEN_NAME);

        let first = load_or_create_token(&path).expect("a token is minted");
        let second = load_or_create_token(&path).expect("the token is reused");

        assert_eq!(
            first, second,
            "restarting must not lock existing clients out"
        );
        assert_eq!(first.len(), 64, "256 bits, hex encoded");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_tokens_are_not_the_same() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let first = load_or_create_token(&dir.path().join("a")).unwrap();
        let second = load_or_create_token(&dir.path().join("b")).unwrap();
        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn a_token_file_is_readable_by_nobody_else() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(TOKEN_NAME);
        load_or_create_token(&path).expect("a token is minted");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the token is a secret on disk");
    }

    #[test]
    fn a_blank_token_file_is_replaced_rather_than_trusted() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(TOKEN_NAME);
        std::fs::write(&path, "   \n").unwrap();
        let token = load_or_create_token(&path).expect("a token is minted");
        assert_eq!(token.len(), 64, "an empty secret must never authenticate");
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_left_by_a_dead_daemon_is_replaced_not_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(SOCKET_NAME);
        // A plain file standing in for what a crash left behind: nothing is
        // listening on it, so binding must succeed.
        std::fs::write(&path, "").unwrap();
        assert!(
            server::bind_unix(&path).is_ok(),
            "a stale socket must not strand the daemon"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_second_daemon_refuses_to_steal_a_live_socket() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(SOCKET_NAME);
        let _first = server::bind_unix(&path).expect("the first daemon binds");
        assert!(
            server::bind_unix(&path).is_err(),
            "taking over a live socket would silently steal its clients"
        );
    }
}
