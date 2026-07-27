//! Getting bytes to and from a control client: who may connect, how a request
//! is framed, and how replies and pushed events share one socket.
//!
//! One thread per connection, which is noise next to the thousand-odd peer
//! threads the client already runs. Each connection owns a writer thread fed by
//! a channel, so request replies and events interleave without a lock and
//! neither can block the other.

use super::super::proto::{
    AuthParams, CODE_INVALID_REQUEST, CODE_METHOD_NOT_FOUND, CODE_PARSE,
    Method, Request, Response, RpcError,
};
use super::Daemon;
use crate::output::Exit;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

/// How long a failed authentication costs the caller. Not a serious defence —
/// the token is 256 bits — but it keeps a wrong-token loop off the CPU.
const AUTH_FAILURE_DELAY: Duration = Duration::from_secs(1);

/// Replies waiting to go out on one connection. Bounded, so a client that has
/// stopped reading cannot make the daemon buffer without limit.
const WRITE_QUEUE_DEPTH: usize = 1024;

/// The longest request line the daemon will read. Generous next to the largest
/// real request (a `shares.set` naming every folder) and small enough that an
/// unauthenticated peer cannot make the daemon allocate without bound.
const MAX_LINE: u64 = 1 << 20;

/// How long an unauthenticated connection may stay silent. Applies only until
/// `auth` succeeds — an established client is expected to idle, holding its
/// connection open for events without sending anything.
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait before accepting again after a failure.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// How many control connections may be open at once. Far above any real use
/// (a TUI, a few scripts, an agent), and low enough that an unauthenticated
/// peer cannot exhaust the daemon's threads.
const MAX_CONNECTIONS: usize = 64;

/// Where control connections arrive.
///
/// The Unix socket is the local path — its file permissions are the
/// authentication, the same bargain Docker and ssh-agent make. TCP is opt-in
/// and always needs the token.
pub enum Listener {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixListener),
    Tcp(TcpListener),
}

pub enum Stream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_clone().map(Self::Unix),
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) {
        let _ = match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_read_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        };
    }

    /// End the connection from another thread. This is what makes "a client
    /// that stops reading is disconnected" true rather than aspirational: the
    /// hub can only reach a stalled client by closing its socket.
    pub(super) fn shutdown(&self) {
        let _ = match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.shutdown(std::net::Shutdown::Both),
            Self::Tcp(stream) => stream.shutdown(std::net::Shutdown::Both),
        };
    }

    /// Whether the transport itself proves who is calling.
    const fn trusted_by_transport(&self) -> bool {
        match self {
            #[cfg(unix)]
            Self::Unix(_) => true,
            Self::Tcp(_) => false,
        }
    }
}

impl std::io::Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
            Self::Tcp(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

impl Listener {
    /// Accept forever, handing each connection to its own thread.
    pub fn serve(self, daemon: Arc<Daemon>) {
        loop {
            match self.accept() {
                Ok(stream) => spawn_connection(stream, Arc::clone(&daemon)),
                Err(e) => {
                    soulseek_rs::warn!("control accept failed: {e}");
                    // Out of file descriptors does not clear by the next
                    // instruction; retrying flat out spins a core.
                    std::thread::sleep(ACCEPT_BACKOFF);
                }
            }
        }
    }

    fn accept(&self) -> std::io::Result<Stream> {
        match self {
            #[cfg(unix)]
            Self::Unix(listener) => {
                listener.accept().map(|(stream, _)| Stream::Unix(stream))
            }
            Self::Tcp(listener) => {
                listener.accept().map(|(stream, _)| Stream::Tcp(stream))
            }
        }
    }
}

fn spawn_connection(stream: Stream, daemon: Arc<Daemon>) {
    let open = daemon.open.fetch_add(1, Ordering::Relaxed) + 1;
    if open > MAX_CONNECTIONS {
        daemon.open.fetch_sub(1, Ordering::Relaxed);
        soulseek_rs::warn!(
            "refusing a control connection: {MAX_CONNECTIONS} already open. \
             This is a leak unless you really have that many clients — please \
             report it with `soulseek-rs daemon status`."
        );
        stream.shutdown();
        return;
    }

    // A handle kept back so the connection can be closed if it never gets a
    // thread to serve it.
    let refused = stream.try_clone();
    let slots = Arc::clone(&daemon.open);

    // `Builder` rather than `thread::spawn`, which panics when the OS will not
    // give us a thread. That panic would unwind through the accept loop and
    // take the listener with it, so one exhausted moment on a busy client
    // would end control for good. Refusing this one connection recovers.
    let spawned = std::thread::Builder::new()
        .name("soulseek-control".to_string())
        .spawn(move || {
            if let Err(e) = handle(stream, daemon.as_ref()) {
                // Loud, not debug: a client sees only "the connection closed",
                // so this line is the only account of why it did.
                soulseek_rs::warn!(
                    "control connection dropped before it could be served: {e}"
                );
            }
            daemon.open.fetch_sub(1, Ordering::Relaxed);
        });

    if let Err(e) = spawned {
        soulseek_rs::warn!("cannot serve a control connection: {e}");
        if let Ok(refused) = refused {
            refused.shutdown();
        }
        slots.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Serve one connection: authenticate, then answer requests until it closes.
fn handle(stream: Stream, daemon: &Daemon) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let trusted = stream.trusted_by_transport();
    let closer = stream.try_clone()?;

    // A peer that connects and says nothing must not hold a thread forever.
    stream.set_read_timeout(Some(AUTH_TIMEOUT));

    let (out, outbox) = sync_channel::<Response>(WRITE_QUEUE_DEPTH);
    let writer = std::thread::spawn(move || write_loop(stream, &outbox));

    if !authenticate(&mut reader, daemon, trusted, &out) {
        // Let the writer flush the refusal before closing: shutting the socket
        // first discards the reply, and the caller learns only that the
        // connection vanished — which is indistinguishable from the daemon
        // having crashed.
        drop(out);
        let _ = writer.join();
        closer.shutdown();
        return Ok(());
    }
    // An established client is expected to idle, holding the connection open
    // for events without sending anything.
    closer.set_read_timeout(None);

    // Only now does this connection get to see events: an unauthenticated
    // socket must learn nothing at all.
    let (subscription, events) = {
        let closer = closer.try_clone()?;
        daemon.hub.subscribe(move || closer.shutdown())
    };
    let pump = {
        let out = out.clone();
        std::thread::spawn(move || {
            for event in events {
                if out.send(event).is_err() {
                    break;
                }
            }
        })
    };

    // Requests are answered on this thread, in order. A method slow enough to
    // matter (re-indexing a large share) delays only the connection that asked
    // for it, which is the right person to wait — and every other client has a
    // connection of its own. Spawning per request would instead churn a thread
    // for each of the handful of polls a TUI makes every frame.
    //
    // Errors break rather than propagate: returning here would skip the
    // unsubscribe below and leak a hub subscriber plus two threads for the
    // life of the daemon.
    loop {
        match read_line(&mut reader) {
            Ok(Some(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(response) = answer(&line, daemon)
                    && out.send(response).is_err()
                {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                soulseek_rs::debug!("control connection ended: {e}");
                break;
            }
        }
    }

    daemon.hub.unsubscribe(subscription);
    drop(out);
    let _ = pump.join();
    let _ = writer.join();
    closer.shutdown();
    Ok(())
}

/// Read one newline-delimited request, refusing a line long enough to be an
/// attack rather than a request. `Ok(None)` means the peer hung up.
fn read_line(
    reader: &mut BufReader<Stream>,
) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let read = reader.by_ref().take(MAX_LINE).read_line(&mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if read as u64 == MAX_LINE && !line.ends_with('\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request line exceeds the maximum size",
        ));
    }
    Ok(Some(line))
}

/// The first request has to be `auth`, and has to be good.
fn authenticate(
    reader: &mut BufReader<Stream>,
    daemon: &Daemon,
    trusted: bool,
    out: &SyncSender<Response>,
) -> bool {
    let Ok(Some(line)) = read_line(reader) else {
        return false;
    };
    let request: Request = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(e) => {
            let _ = out.send(Response::failure(
                None,
                RpcError::new(CODE_PARSE, e.to_string()),
            ));
            return false;
        }
    };

    if Method::from_wire(&request.method) != Some(Method::Auth) {
        let _ = out.send(Response::failure(
            request.id,
            RpcError::new(
                CODE_INVALID_REQUEST,
                "the first request on a connection must be `auth`",
            ),
        ));
        return false;
    }

    let params: AuthParams =
        serde_json::from_value(request.params).unwrap_or_default();
    if !accepts(daemon, trusted, params.token.as_deref()) {
        std::thread::sleep(AUTH_FAILURE_DELAY);
        let _ = out.send(Response::failure(
            request.id,
            RpcError::application(Exit::Connection, "authentication failed"),
        ));
        return false;
    }

    let result = serde_json::to_value(daemon.identify()).unwrap_or(Value::Null);
    let _ = out.send(Response::result(request.id, result));
    true
}

/// A Unix socket has already proved the caller is this user, so a token is
/// optional there and mandatory everywhere else. A token that *is* offered is
/// always checked, so a wrong one never passes.
pub(super) fn accepts(
    daemon: &Daemon,
    trusted: bool,
    token: Option<&str>,
) -> bool {
    match token {
        Some(token) => constant_time_eq(token, &daemon.token),
        None => trusted,
    }
}

/// Compare without leaking the position of the first difference through timing.
pub(super) fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// Turn one request line into the reply it deserves, or `None` for a
/// notification, which by JSON-RPC rule is answered with silence.
pub(super) fn answer(line: &str, daemon: &Daemon) -> Option<Response> {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => {
            return Some(Response::failure(
                None,
                RpcError::new(CODE_PARSE, e.to_string()),
            ));
        }
    };
    let id = request.id.clone();
    let Some(method) = Method::from_wire(&request.method) else {
        return id.is_some().then(|| {
            Response::failure(
                request.id,
                RpcError::new(
                    CODE_METHOD_NOT_FOUND,
                    format!("no such method: {}", request.method),
                ),
            )
        });
    };

    let outcome = daemon.call(method, request.params);
    id.map(|id| match outcome {
        Ok(result) => Response::result(Some(id), result),
        Err(error) => Response::failure(Some(id), error),
    })
}

fn write_loop(stream: Stream, outbox: &Receiver<Response>) {
    let mut stream = stream;
    for response in outbox {
        let Ok(line) = serde_json::to_string(&response) else {
            continue;
        };
        if writeln!(stream, "{line}").is_err() || stream.flush().is_err() {
            return;
        }
    }
}

/// Bind the TCP control port, refusing to come up rather than silently
/// listening somewhere unintended.
pub fn bind_tcp(address: &str) -> std::io::Result<Listener> {
    TcpListener::bind(address).map(Listener::Tcp)
}

/// Bind the Unix control socket with permissions that make it the local
/// authentication: only this user can open it.
///
/// Bound under a temporary name and moved into place, because `bind` creates
/// the socket with the process umask — for the moment between `bind` and
/// `set_permissions` the real path would be connectable by anyone on the
/// machine, and on this transport connecting *is* authenticating. The rename
/// is atomic, so the published path never exists with the wrong mode.
#[cfg(unix)]
pub fn bind_unix(path: &std::path::Path) -> std::io::Result<Listener> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A socket file outlives the process that made it, so a previous crash
    // would otherwise make the daemon unstartable.
    if path.exists() {
        remove_stale_socket(path)?;
    }

    let staging = path.with_extension("binding");
    let _ = std::fs::remove_file(&staging);
    let listener = std::os::unix::net::UnixListener::bind(&staging)?;
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&staging, path)?;
    Ok(Listener::Unix(listener))
}

/// Remove a socket file left by a dead daemon — but never one a live daemon is
/// still serving, which would silently steal its clients.
#[cfg(unix)]
fn remove_stale_socket(path: &std::path::Path) -> std::io::Result<()> {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            format!("a daemon is already listening on {}", path.display()),
        )),
        Err(_) => std::fs::remove_file(path),
    }
}
