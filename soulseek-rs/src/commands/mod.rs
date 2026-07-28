//! One-shot commands: everything the binary can do without a terminal.
//!
//! Each command does one job, writes records to stdout and progress to stderr,
//! and returns an exit code through [`CliError`]. Most open a [`Session`]
//! first; the ones that only read the config file or the local machine are
//! dispatched by `main::run` before any credentials are demanded.

pub mod completions;
pub mod daemon;
pub mod peer;
pub mod settings;
pub mod skills;
pub mod social;
pub mod transfer;
pub mod wish;

use crate::api::SessionApi;
use crate::cli::{Commands, MessageCommand, RoomCommand};
use crate::output::{CliError, CliResult, Exit, Out, PortmapRecord};
use crate::port_mapping::{self, PortMapper};
use crate::remote::{Endpoint, RemoteSession};
use soulseek_rs::types::{RoomEvent, RoomInfo};
use soulseek_rs::{Client, ClientSettings, SessionLoss};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Everything a one-shot command needs that does not come from its own flags.
pub struct Ctx {
    pub out: Out,
    pub settings: ClientSettings,
    pub download_dir: String,
    pub max_concurrent_downloads: usize,
    pub search_timeout: Duration,
    /// Where to find a daemon, when this run should use one rather than open
    /// its own session. Resolved once in `main`, so every command agrees.
    pub daemon: Option<Endpoint>,
    /// Token for a remote daemon; a Unix socket needs none.
    pub daemon_token: Option<String>,
    /// The client of the session this command opened, once it has one, so the
    /// verdict can be checked against the session that produced it.
    pub session: OnceLock<Arc<dyn SessionApi>>,
}

impl Ctx {
    /// Re-read a "found nothing", "timed out" or "transfer failed" verdict
    /// against the session that produced it.
    ///
    /// A session the server cut off — most often because another process
    /// logged in under the same name — sees nothing at all from that moment
    /// on. Reported as exit 4 it reads as "this file is not on the network",
    /// and a caller acting on that gives up on files that were there.
    fn checked(&self, result: CliResult) -> CliResult {
        let Err(error) = result else {
            return Ok(());
        };
        if !matches!(
            error.exit,
            Exit::NoResults | Exit::Timeout | Exit::Transfer
        ) {
            return Err(error);
        }
        match self.session.get().and_then(|client| client.session_loss()) {
            Some(loss) => Err(session_lost(loss)),
            None => Err(error),
        }
    }
}

/// What to tell a caller whose session ended under it.
fn session_lost(loss: SessionLoss) -> CliError {
    let fix = match loss {
        SessionLoss::Displaced => {
            "; give each concurrent run its own --username"
        }
        SessionLoss::Disconnected => "",
    };
    CliError::session_lost(format!(
        "the session ended before this command finished: {loss}{fix}"
    ))
}

/// A logged-in client plus, while the listener is enabled, the port mapping
/// that makes us reachable. Both live until the command ends.
pub struct Session {
    pub client: Arc<dyn SessionApi>,
    _mapper: Option<PortMapper>,
}

/// How long to wait for the TCP handshake with the server.
const REACH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the login exchange once the socket is open.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

impl Session {
    /// Get a session: the daemon's, if this run is routed to one, or a fresh
    /// login of its own.
    pub fn open(ctx: &Ctx) -> CliResult<Self> {
        match &ctx.daemon {
            Some(endpoint) => Self::attach(ctx, endpoint),
            None => Self::login(ctx),
        }
    }

    /// Borrow the daemon's session instead of logging in again.
    ///
    /// The server allows one login per account, so a second login would
    /// displace the daemon — attaching is not just cheaper, it is the only way
    /// two things can be on the network as the same user at once.
    fn attach(ctx: &Ctx, endpoint: &Endpoint) -> CliResult<Self> {
        ctx.out.status(&format!("using the daemon at {endpoint}"));
        let token = ctx
            .daemon_token
            .clone()
            .or_else(crate::daemon::stored_token);
        let remote = RemoteSession::connect(endpoint, token).map_err(|e| {
            CliError::connection(format!("cannot use the daemon: {e}"))
        })?;

        let client: Arc<dyn SessionApi> = Arc::new(remote);
        ctx.out
            .status(&format!("logged in as {}", client.username()));
        let _ = ctx.session.set(Arc::clone(&client));
        Ok(Self {
            client,
            // The daemon holds the listener, so it owns the port mapping too.
            _mapper: None,
        })
    }

    fn login(ctx: &Ctx) -> CliResult<Self> {
        let address = ctx.settings.server_address.to_string();
        ctx.out.status(&format!("connecting to {address}…"));
        reachable(&address, REACH_TIMEOUT)?;

        let client: Arc<dyn SessionApi> =
            Arc::new(login(ctx.settings.clone(), &address)?);
        ctx.out
            .status(&format!("logged in as {}", ctx.settings.username));
        let _ = ctx.session.set(Arc::clone(&client));

        let bound = client.listen_port();
        if let Some(bound) = bound
            && ctx.settings.listen_port != 0
            && bound != ctx.settings.listen_port
        {
            ctx.out.warn(&format!(
                "listener port {} is in use; listening on {bound} instead",
                ctx.settings.listen_port
            ));
        }

        // Ask the router for the port the listener actually holds: when the
        // configured one was taken, mapping it would open a hole to whoever
        // did take it.
        let mapper = bound.map(PortMapper::spawn);

        Ok(Self {
            client,
            _mapper: mapper,
        })
    }
}

/// Check the server answers at all before handing the socket to the client.
///
/// The connect and login the library performs happen on its own threads and
/// report a refused connection only indirectly, so an unattended run would sit
/// there. One handshake up front turns that into an immediate, accurate error.
fn reachable(address: &str, timeout: Duration) -> CliResult {
    let resolved: Vec<SocketAddr> = address
        .to_socket_addrs()
        .map_err(|e| {
            CliError::connection(format!("cannot resolve {address}: {e}"))
        })?
        .collect();

    let mut last = None;
    for candidate in &resolved {
        match TcpStream::connect_timeout(candidate, timeout) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(CliError::connection(match last {
        Some(e) => format!("cannot reach {address}: {e}"),
        None => format!("cannot reach {address}: no address resolved"),
    }))
}

/// Connect and log in under a deadline.
///
/// `Client::login` blocks on a channel the library only answers once its actor
/// has a working connection, so it is run on its own thread: a server that
/// accepts the socket and then says nothing must still end the command.
fn login(settings: ClientSettings, address: &str) -> CliResult<Client> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut client = Client::with_settings(settings);
        let outcome = match client.connect() {
            Ok(()) => client.login(),
            Err(e) => Err(e),
        };
        let _ = sender.send((client, outcome));
    });

    let (client, outcome) =
        receiver.recv_timeout(LOGIN_TIMEOUT).map_err(|_| {
            CliError::connection(format!(
                "{address} accepted the connection but did not answer the \
                 login within {}s",
                LOGIN_TIMEOUT.as_secs()
            ))
        })?;

    match outcome {
        Ok(true) => Ok(client),
        Ok(false) => Err(CliError::connection(
            "login rejected by the server (wrong password, or the name is \
             taken)",
        )),
        Err(e) => Err(CliError::connection(format!("login failed: {e}"))),
    }
}

/// Ask `f` every `poll` until it answers or `timeout` runs out.
///
/// The library reports what the network sent by handing it over on the next
/// call, so a command that wants one answer has to keep asking. Checking first
/// and sleeping after means an answer already waiting costs nothing.
fn poll_until<T>(
    timeout: Duration,
    poll: Duration,
    mut f: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(answer) = f() {
            return Some(answer);
        }
        std::thread::sleep(poll);
    }
    None
}

/// Ask for the room list and wait for the server's reply.
///
/// Events already queued are discarded first: servers push a room list at
/// login, and counting that one would answer a question we had not asked yet.
pub fn await_room_list(
    client: &dyn SessionApi,
    timeout: Duration,
    poll: Duration,
) -> Option<Vec<RoomInfo>> {
    let _ = client.take_room_events();
    client.request_room_list().ok()?;

    poll_until(timeout, poll, || {
        client
            .take_room_events()
            .into_iter()
            .find_map(|event| match event {
                RoomEvent::List(rooms) => Some(rooms),
                _ => None,
            })
    })
}

/// Wait until the server has answered something we asked for after `send`.
///
/// The server connection is a single ordered stream served by one actor, so a
/// reply to a request queued *after* an earlier message proves that earlier
/// message reached the socket. The room list is the cheapest such request and
/// every server answers it, which lets `message send` and `room say` report
/// delivery honestly instead of sleeping and hoping.
pub fn confirm_flush(client: &dyn SessionApi, timeout: Duration) -> bool {
    await_room_list(client, timeout, Duration::from_millis(50)).is_some()
}

pub fn run(ctx: &Ctx, command: Commands) -> CliResult {
    ctx.checked(dispatch(ctx, command))
}

fn dispatch(ctx: &Ctx, command: Commands) -> CliResult {
    match command {
        Commands::Search(args) => transfer::search(ctx, &args),
        Commands::Download(args) => transfer::download(ctx, &args),
        Commands::Get(args) => transfer::get(ctx, &args),
        Commands::Browse(args) => social::browse(ctx, &args),
        Commands::Room(RoomCommand::List { timeout }) => {
            social::room_list(ctx, Duration::from_secs(timeout))
        }
        Commands::Room(RoomCommand::Say { room, message }) => {
            social::room_say(ctx, &room, &message)
        }
        Commands::Room(RoomCommand::Users { room, timeout }) => {
            social::room_users(ctx, &room, Duration::from_secs(timeout))
        }
        Commands::Room(RoomCommand::Listen {
            room,
            duration,
            follow,
        }) => social::room_listen(ctx, &room, listen_span(duration, follow)),
        Commands::Message(MessageCommand::Send { user, message }) => {
            social::message_send(ctx, &user, &message)
        }
        Commands::Message(MessageCommand::Read { duration, follow }) => {
            social::message_read(ctx, listen_span(duration, follow))
        }
        Commands::User(args) => peer::user(ctx, &args),
        Commands::Whoami => peer::whoami(ctx),
        // main.rs runs these itself: some need no account, the rest need the
        // config store in scope. The arms only keep the match exhaustive.
        Commands::Portmap
        | Commands::Serve(_)
        | Commands::Daemon(_)
        | Commands::Wish(_)
        | Commands::Config(_)
        | Commands::Shares(_)
        | Commands::Skills(_)
        | Commands::Completions(_) => {
            Err(CliError::usage("handled without a session; see main::run"))
        }
    }
}

/// `--follow` means "until interrupted": a span nothing will outlive.
fn listen_span(duration: u64, follow: bool) -> Option<Duration> {
    (!follow).then(|| Duration::from_secs(duration))
}

/// Where shells and coding agents keep their configuration: the XDG layout on
/// every platform, unlike this client's own config, which follows each
/// platform's convention (see `persist::paths`).
fn xdg(variable: &str, home: &Path, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(fallback), PathBuf::from)
}

fn unwritable(path: &Path, error: &std::io::Error) -> CliError {
    CliError::usage(format!("cannot write {}: {error}", path.display()))
}

/// Probe automatic port mapping. A router that does not answer is a negative
/// answer, not a crash, so it exits 4 the way `grep` exits 1 on no match.
pub fn portmap(out: &Out, port: u16) -> CliResult {
    out.status(&format!("probing automatic port mapping for TCP {port}…"));
    let probe = port_mapping::probe(port);
    out.emit(&PortmapRecord {
        ok: probe.ok,
        backend: probe.backend.to_string(),
        external: probe.external.clone(),
        port,
    });
    out.status(&probe.advice);
    if probe.ok {
        Ok(())
    } else {
        // The advice is already on stderr; the error line only has to say
        // what the exit code means.
        Err(CliError::new(
            Exit::NoResults,
            format!("no router mapped TCP {port}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::listen_span;
    use std::time::Duration;

    #[test]
    fn follow_listens_without_a_deadline() {
        assert_eq!(listen_span(30, true), None);
        assert_eq!(listen_span(30, false), Some(Duration::from_secs(30)));
    }
}
