//! One-shot commands: everything the binary can do without a terminal.
//!
//! Each command opens a [`Session`], does one job, writes records to stdout
//! and progress to stderr, and returns an exit code through [`CliError`].

pub mod social;
pub mod transfer;

use crate::cli::{Commands, MessageCommand, RoomCommand};
use crate::output::{CliError, CliResult, Exit, Out, PortmapRecord};
use crate::port_mapping::{self, PortMapper};
use soulseek_rs::types::{RoomEvent, RoomInfo};
use soulseek_rs::{Client, ClientSettings};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Everything a one-shot command needs that does not come from its own flags.
pub struct Ctx {
    pub out: Out,
    pub settings: ClientSettings,
    pub download_dir: String,
    pub max_concurrent_downloads: usize,
    pub search_timeout: Duration,
}

/// A logged-in client plus, while the listener is enabled, the port mapping
/// that makes us reachable. Both live until the command ends.
pub struct Session {
    pub client: Arc<Client>,
    _mapper: Option<PortMapper>,
}

/// How long to wait for the TCP handshake with the server.
const REACH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the login exchange once the socket is open.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

impl Session {
    pub fn open(ctx: &Ctx) -> CliResult<Self> {
        let mapper = ctx
            .settings
            .enable_listen
            .then(|| PortMapper::spawn(ctx.settings.listen_port));

        let address = ctx.settings.server_address.to_string();
        ctx.out.status(&format!("connecting to {address}…"));
        reachable(&address, REACH_TIMEOUT)?;

        let client = login(ctx.settings.clone(), &address)?;
        ctx.out
            .status(&format!("logged in as {}", ctx.settings.username));

        Ok(Self {
            client: Arc::new(client),
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

/// Ask for the room list and wait for the server's reply.
///
/// Events already queued are discarded first: servers push a room list at
/// login, and counting that one would answer a question we had not asked yet.
pub fn await_room_list(
    client: &Client,
    timeout: Duration,
    poll: Duration,
) -> Option<Vec<RoomInfo>> {
    let _ = client.take_room_events();
    client.request_room_list().ok()?;

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let listed =
            client.take_room_events().into_iter().find_map(
                |event| match event {
                    RoomEvent::List(rooms) => Some(rooms),
                    _ => None,
                },
            );
        if listed.is_some() {
            return listed;
        }
        std::thread::sleep(poll);
    }
    None
}

/// Wait until the server has answered something we asked for after `send`.
///
/// The server connection is a single ordered stream served by one actor, so a
/// reply to a request queued *after* an earlier message proves that earlier
/// message reached the socket. The room list is the cheapest such request and
/// every server answers it, which lets `message send` and `room say` report
/// delivery honestly instead of sleeping and hoping.
pub fn confirm_flush(client: &Client, timeout: Duration) -> bool {
    await_room_list(client, timeout, Duration::from_millis(50)).is_some()
}

pub fn run(ctx: &Ctx, command: Commands) -> CliResult {
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
        // Handled before credentials are required; see main.rs.
        Commands::Portmap => portmap(&ctx.out, ctx.settings.listen_port),
    }
}

/// `--follow` means "until interrupted": a span nothing will outlive.
fn listen_span(duration: u64, follow: bool) -> Option<Duration> {
    (!follow).then(|| Duration::from_secs(duration))
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
