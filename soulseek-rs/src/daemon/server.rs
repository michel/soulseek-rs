//! The control listener: who may connect, and what they may ask for.
//!
//! One thread per connection, which is noise next to the thousand-odd peer
//! threads the client already runs. Each connection owns a writer thread fed by
//! a channel, so request replies and pushed events interleave onto the socket
//! without a lock and without either being able to block the other.

use super::hub::{Hub, PendingBrowses, PendingDownload};
use super::proto::{
    Ack, AuthParams, AuthResult, CODE_APPLICATION, CODE_INVALID_PARAMS,
    ChatMessageDto,
    CODE_INVALID_REQUEST, CODE_METHOD_NOT_FOUND, CODE_PARSE, DaemonStatus,
    DirectoriesParams, DownloadDto, DownloadStartParams, DownloadStarted,
    Downloads, IntervalSeconds, MessageParams, Members, Messages, Method,
    OPENRPC, PROTOCOL_VERSION, QueryParams, Request, Response, RoomRef,
    RpcError, SayParams, SearchResultDto, SearchResults, Seconds, SharesStatus,
    SlotsParams, TransferRef, UploadInfoDto, Uploads, UserInfoDto, UserRef,
    UserResult,
};
use crate::api::SessionApi;
use crate::output::Exit;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a failed authentication costs the caller. Not a serious defence —
/// the token is 256 bits — but it keeps a wrong-token loop off the CPU.
const AUTH_FAILURE_DELAY: Duration = Duration::from_secs(1);

/// Replies waiting to go out on one connection. Bounded, so a client that has
/// stopped reading cannot make the daemon buffer without limit.
const WRITE_QUEUE_DEPTH: usize = 1024;

/// Everything a connection needs to answer a request.
pub struct Daemon {
    pub session: Arc<dyn SessionApi>,
    pub hub: Arc<Hub>,
    pub browses: Arc<PendingBrowses>,
    pub downloads: Arc<Mutex<Vec<PendingDownload>>>,
    /// Where transfers land, on *this* machine. A remote caller cannot see
    /// this filesystem, so it does not get to choose.
    pub download_dir: String,
    pub server: String,
    pub token: String,
    pub started: Instant,
    pub stop: Arc<AtomicBool>,
    /// Rooms this session has joined, so a restart can rejoin them.
    pub rooms: Mutex<Vec<String>>,
}

impl Daemon {
    /// Answer one request.
    ///
    /// The match is exhaustive over [`Method`], so a new method cannot be added
    /// to the protocol without being implemented here.
    #[allow(clippy::too_many_lines)]
    pub fn call(&self, method: Method, params: Value) -> Result<Value, RpcError> {
        match method {
            // Handled during the handshake; reaching here means a second auth.
            Method::Auth => ok(Ack::OK),
            Method::DaemonStatus => ok(self.status()),
            Method::DaemonStop => {
                self.stop.store(true, Ordering::Relaxed);
                ok(Ack::OK)
            }
            Method::RpcDiscover => serde_json::from_str(OPENRPC)
                .map_err(|e| internal(format!("the contract is unreadable: {e}"))),

            Method::SearchStart => {
                let query: QueryParams = parse(params)?;
                // Zero window: put the search on the wire and return. The
                // caller decides how long to let answers accumulate, which is
                // what keeps one client's search from blocking the daemon.
                self.session
                    .search_with_cancel(&query.query, Duration::ZERO, None)
                    .map_err(|e| failed(Exit::Connection, format!("search failed: {e}")))?;
                ok(Ack::OK)
            }
            Method::SearchResults => {
                let query: QueryParams = parse(params)?;
                ok(SearchResults {
                    results: self
                        .session
                        .get_search_results(&query.query)
                        .iter()
                        .map(SearchResultDto::from)
                        .collect(),
                })
            }
            Method::SearchWishlist => {
                let query: QueryParams = parse(params)?;
                self.session
                    .start_wishlist_search(&query.query)
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                ok(Ack::OK)
            }
            Method::SearchWishlistInterval => ok(IntervalSeconds {
                seconds: self.session.wishlist_interval().as_secs(),
            }),

            Method::DownloadStart => self.start_download(parse(params)?),
            Method::DownloadList => ok(Downloads {
                downloads: self
                    .session
                    .get_all_downloads()
                    .iter()
                    .map(DownloadDto::from)
                    .collect(),
            }),
            Method::DownloadPause => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .pause_download(&transfer.username, &transfer.filename),
                })
            }
            Method::DownloadResume => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .resume_download(&transfer.username, &transfer.filename),
                })
            }
            Method::DownloadRemove => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .remove_download(&transfer.username, &transfer.filename),
                })
            }
            Method::DownloadRemoveQueued => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self.session.remove_queued_download(
                        &transfer.username,
                        &transfer.filename,
                    ),
                })
            }

            Method::UploadList => ok(Uploads {
                uploads: self
                    .session
                    .uploads()
                    .iter()
                    .map(UploadInfoDto::from)
                    .collect(),
            }),
            Method::UploadCancel => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .cancel_upload(&transfer.username, &transfer.filename),
                })
            }
            Method::UploadSlots => {
                let slots: SlotsParams = parse(params)?;
                self.session.set_upload_slots(slots.slots);
                ok(Ack::OK)
            }

            Method::PrivilegesCheck => {
                self.session
                    .check_privileges()
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                ok(Ack::OK)
            }
            Method::PrivilegesOwn => ok(Seconds {
                seconds: self.session.own_privilege_seconds(),
            }),

            Method::RoomListRequest => {
                self.session
                    .request_room_list()
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                ok(Ack::OK)
            }
            Method::RoomJoin => {
                let room: RoomRef = parse(params)?;
                self.session
                    .join_room(&room.room)
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                self.joined(&room.room, true);
                ok(Ack::OK)
            }
            Method::RoomLeave => {
                let room: RoomRef = parse(params)?;
                self.session
                    .leave_room(&room.room)
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                self.joined(&room.room, false);
                ok(Ack::OK)
            }
            Method::RoomSay => {
                let say: SayParams = parse(params)?;
                self.session
                    .say_in_room(&say.room, &say.message)
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                ok(Ack::OK)
            }
            Method::RoomMembers => {
                let room: RoomRef = parse(params)?;
                ok(Members {
                    users: self.session.room_members(&room.room),
                })
            }

            Method::MessageSend => {
                let message: MessageParams = parse(params)?;
                self.session
                    .send_private_message(&message.username, &message.message)
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                // Record our own half of the conversation: the drainer only
                // ever sees incoming messages, so without this a client that
                // attaches later reads a one-sided chat.
                self.hub.remember(ChatMessageDto {
                    peer: message.username,
                    outgoing: true,
                    text: message.message,
                    at: now_seconds(),
                });
                ok(Ack::OK)
            }
            Method::MessageHistory => ok(Messages {
                messages: self.hub.history(),
            }),

            Method::BrowseUser => {
                let user: UserRef = parse(params)?;
                // Register before asking: the answer can land on the very next
                // drainer tick, and an unexpected listing is discarded.
                self.browses.expect(&user.username);
                if let Err(e) = self.session.browse_user(&user.username) {
                    self.browses.forget(&user.username);
                    return Err(failed(Exit::Connection, e.to_string()));
                }
                ok(Ack::OK)
            }
            Method::UserRequest => {
                let user: UserRef = parse(params)?;
                self.session
                    .request_user_info(&user.username)
                    .map_err(|e| failed(Exit::Connection, e.to_string()))?;
                ok(Ack::OK)
            }
            Method::UserInfoOf => {
                let user: UserRef = parse(params)?;
                ok(UserResult {
                    user: self
                        .session
                        .user_info(&user.username)
                        .as_ref()
                        .map(UserInfoDto::from),
                })
            }

            Method::SharesStatusOf => ok(self.shares()),
            Method::SharesSet => {
                let directories: DirectoriesParams = parse(params)?;
                self.session
                    .set_shared_directories(directories.directories)
                    .map_err(|e| failed(Exit::Usage, e.to_string()))?;
                ok(self.shares())
            }
            Method::SharesReindex => {
                self.session
                    .set_shared_directories(self.session.shared_directories())
                    .map_err(|e| failed(Exit::Usage, e.to_string()))?;
                ok(self.shares())
            }
        }
    }

    /// Track membership so a restart can rejoin what was open. The library
    /// has no "which rooms am I in" accessor, so the daemon keeps the list.
    fn joined(&self, room: &str, member: bool) {
        let Ok(mut rooms) = self.rooms.lock() else {
            return;
        };
        rooms.retain(|name| name != room);
        if member {
            rooms.push(room.to_string());
        }
    }

    #[must_use]
    pub fn rooms(&self) -> Vec<String> {
        self.rooms
            .lock()
            .map_or_else(|_| Vec::new(), |rooms| rooms.clone())
    }

    fn start_download(
        &self,
        params: DownloadStartParams,
    ) -> Result<Value, RpcError> {
        let directory = params
            .download_directory
            .unwrap_or_else(|| self.download_dir.clone());
        let (download, updates) = self
            .session
            .download_with_metadata(
                params.filename.clone(),
                params.username.clone(),
                params.size,
                directory,
                params.metadata.into(),
            )
            .map_err(|e| failed(Exit::Transfer, format!("cannot start: {e}")))?;

        if let Ok(mut pending) = self.downloads.lock() {
            pending.push(PendingDownload {
                username: params.username,
                filename: params.filename,
                updates,
            });
        }
        ok(DownloadStarted {
            download: DownloadDto::from(&download),
        })
    }

    fn shares(&self) -> SharesStatus {
        let (folders, files) = self.session.shared_counts();
        SharesStatus {
            folders,
            files,
            directories: self.session.shared_directories(),
        }
    }

    fn status(&self) -> DaemonStatus {
        let (folders, files) = self.session.shared_counts();
        DaemonStatus {
            username: self.session.username(),
            server: self.server.clone(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            listen_port: self.session.listen_port(),
            shared_folders: folders,
            shared_files: files,
            download_dir: self.download_dir.clone(),
            session_loss: self.session.session_loss().map(Into::into),
            clients: self.hub.subscribers(),
            uptime_secs: self.started.elapsed().as_secs(),
        }
    }
}

fn ok<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|e| internal(format!("cannot serialize the reply: {e}")))
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|e| RpcError::new(CODE_INVALID_PARAMS, e.to_string()))
}

fn failed(exit: Exit, message: impl Into<String>) -> RpcError {
    RpcError::application(exit, message)
}

fn internal(message: impl Into<String>) -> RpcError {
    RpcError::new(CODE_APPLICATION, message)
}

/// Wall-clock seconds, to stamp a message the way the server stamps the ones
/// it delivers. A clock before the epoch reads as zero rather than panicking.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

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
        match self {
            #[cfg(unix)]
            Self::Unix(listener) => {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => spawn_connection(
                            Stream::Unix(stream),
                            Arc::clone(&daemon),
                        ),
                        Err(e) => {
                            soulseek_rs::warn!("control accept failed: {e}");
                        }
                    }
                }
            }
            Self::Tcp(listener) => {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => spawn_connection(
                            Stream::Tcp(stream),
                            Arc::clone(&daemon),
                        ),
                        Err(e) => {
                            soulseek_rs::warn!("control accept failed: {e}");
                        }
                    }
                }
            }
        }
    }
}

fn spawn_connection(stream: Stream, daemon: Arc<Daemon>) {
    std::thread::spawn(move || {
        if let Err(e) = handle(stream, daemon) {
            soulseek_rs::debug!("control connection ended: {e}");
        }
    });
}

/// Serve one connection: authenticate, then answer requests until it closes.
fn handle(stream: Stream, daemon: Arc<Daemon>) -> std::io::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let trusted = stream.trusted_by_transport();

    let (out, outbox) = sync_channel::<Response>(WRITE_QUEUE_DEPTH);
    let writer = std::thread::spawn(move || write_loop(stream, &outbox));

    let mut lines = reader.lines();
    let authenticated = authenticate(&mut lines, &daemon, trusted, &out);
    if !authenticated {
        drop(out);
        let _ = writer.join();
        return Ok(());
    }

    // Only now does this connection get to see events: an unauthenticated
    // socket must learn nothing at all.
    let (subscription, events) = daemon.hub.subscribe();
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

    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let daemon = Arc::clone(&daemon);
        let out = out.clone();
        // One thread per request: a slow method must not hold up the ones
        // behind it, and the ids let a client match replies out of order.
        std::thread::spawn(move || {
            if let Some(response) = answer(&line, &daemon) {
                let _ = out.send(response);
            }
        });
    }

    daemon.hub.unsubscribe(subscription);
    drop(out);
    let _ = pump.join();
    let _ = writer.join();
    Ok(())
}

/// The first request has to be `auth`, and has to be good.
fn authenticate(
    lines: &mut std::io::Lines<BufReader<Stream>>,
    daemon: &Daemon,
    trusted: bool,
    out: &SyncSender<Response>,
) -> bool {
    let Some(Ok(line)) = lines.next() else {
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

    let result = serde_json::to_value(AuthResult {
        protocol: PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        username: daemon.session.username(),
    })
    .unwrap_or(Value::Null);
    let _ = out.send(Response::result(request.id, result));
    true
}

/// A Unix socket has already proved the caller is this user, so a token is
/// optional there and mandatory everywhere else. A token that *is* offered is
/// always checked, so a wrong one never passes.
fn accepts(daemon: &Daemon, trusted: bool, token: Option<&str>) -> bool {
    match token {
        Some(token) => constant_time_eq(token, &daemon.token),
        None => trusted,
    }
}

/// Compare without leaking the position of the first difference through timing.
fn constant_time_eq(left: &str, right: &str) -> bool {
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
fn answer(line: &str, daemon: &Daemon) -> Option<Response> {
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
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_unix_socket_needs_no_token_but_tcp_does() {
        let daemon = daemon_with_token("secret");
        assert!(accepts(&daemon, true, None), "socket permissions are the auth");
        assert!(!accepts(&daemon, false, None), "TCP must demand a token");
    }

    #[test]
    fn a_wrong_token_is_refused_on_every_transport() {
        let daemon = daemon_with_token("secret");
        assert!(!accepts(&daemon, false, Some("guess")));
        assert!(
            !accepts(&daemon, true, Some("guess")),
            "a token that is offered is always checked, even locally"
        );
        assert!(accepts(&daemon, false, Some("secret")));
    }

    #[test]
    fn token_comparison_does_not_short_circuit_on_length_or_content() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn an_unknown_method_is_reported_rather_than_ignored() {
        let daemon = daemon_with_token("t");
        let response = answer(
            r#"{"jsonrpc":"2.0","id":1,"method":"search.nope"}"#,
            &daemon,
        )
        .expect("a request with an id is always answered");
        let error = response.error.expect("an unknown method is an error");
        assert_eq!(error.code, CODE_METHOD_NOT_FOUND);
    }

    #[test]
    fn a_notification_gets_no_reply_even_when_it_fails() {
        let daemon = daemon_with_token("t");
        assert!(
            answer(r#"{"jsonrpc":"2.0","method":"search.nope"}"#, &daemon)
                .is_none(),
            "a request without an id must be answered with silence"
        );
    }

    #[test]
    fn unparseable_input_is_a_parse_error_not_a_dropped_connection() {
        let daemon = daemon_with_token("t");
        let response =
            answer("this is not json", &daemon).expect("garbage is answered");
        assert_eq!(response.error.expect("an error").code, CODE_PARSE);
    }

    #[test]
    fn bad_params_are_reported_as_such() {
        let daemon = daemon_with_token("t");
        let error = daemon
            .call(Method::RoomJoin, json!({ "not_a_room": 1 }))
            .expect_err("the params do not match");
        assert_eq!(error.code, CODE_INVALID_PARAMS);
    }

    #[test]
    fn discover_serves_the_published_contract() {
        let daemon = daemon_with_token("t");
        let document = daemon
            .call(Method::RpcDiscover, Value::Null)
            .expect("the contract is compiled in");
        assert_eq!(document["openrpc"], "1.3.2");
        assert!(
            document["methods"]
                .as_array()
                .is_some_and(|methods| methods.len() == Method::ALL.len()
                    + super::super::proto::Event::ALL.len()),
            "discover must describe every method and event"
        );
    }

    #[test]
    fn stop_is_recorded_so_the_daemon_can_wind_down() {
        let daemon = daemon_with_token("t");
        assert!(!daemon.stop.load(Ordering::Relaxed));
        daemon
            .call(Method::DaemonStop, Value::Null)
            .expect("stop is always accepted");
        assert!(daemon.stop.load(Ordering::Relaxed));
    }

    // A session that answers nothing: enough to exercise dispatch, parameter
    // validation, and the auth gate without a Soulseek server.
    struct SilentSession;

    fn daemon_with_token(token: &str) -> Daemon {
        Daemon {
            session: Arc::new(SilentSession),
            hub: Arc::new(Hub::new()),
            browses: Arc::new(PendingBrowses::default()),
            downloads: Arc::new(Mutex::new(Vec::new())),
            download_dir: "/tmp".to_string(),
            server: "server.invalid:2416".to_string(),
            token: token.to_string(),
            started: Instant::now(),
            stop: Arc::new(AtomicBool::new(false)),
            rooms: Mutex::new(Vec::new()),
        }
    }

    impl SessionApi for SilentSession {
        fn username(&self) -> String {
            "tester".to_string()
        }
        fn listen_port(&self) -> Option<u16> {
            None
        }
        fn session_loss(&self) -> Option<soulseek_rs::SessionLoss> {
            None
        }
        fn search(
            &self,
            _query: &str,
            _timeout: Duration,
        ) -> soulseek_rs::Result<Vec<soulseek_rs::SearchResult>> {
            Ok(Vec::new())
        }
        fn search_with_cancel(
            &self,
            _query: &str,
            _timeout: Duration,
            _cancel: Option<Arc<AtomicBool>>,
        ) -> soulseek_rs::Result<Vec<soulseek_rs::SearchResult>> {
            Ok(Vec::new())
        }
        fn get_search_results(
            &self,
            _key: &str,
        ) -> Vec<soulseek_rs::SearchResult> {
            Vec::new()
        }
        fn get_search_results_count(&self, _key: &str) -> usize {
            0
        }
        fn try_get_search_results(
            &self,
            _key: &str,
        ) -> Option<Vec<soulseek_rs::SearchResult>> {
            None
        }
        fn start_wishlist_search(&self, _query: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn wishlist_interval(&self) -> Duration {
            Duration::from_mins(12)
        }
        fn download(
            &self,
            filename: String,
            username: String,
            size: u64,
            directory: String,
        ) -> soulseek_rs::Result<(
            soulseek_rs::types::Download,
            Receiver<soulseek_rs::DownloadStatus>,
        )> {
            self.download_with_metadata(
                filename,
                username,
                size,
                directory,
                soulseek_rs::types::DownloadMetadata::default(),
            )
        }
        fn download_with_metadata(
            &self,
            _filename: String,
            _username: String,
            _size: u64,
            _directory: String,
            _metadata: soulseek_rs::types::DownloadMetadata,
        ) -> soulseek_rs::Result<(
            soulseek_rs::types::Download,
            Receiver<soulseek_rs::DownloadStatus>,
        )> {
            Err(soulseek_rs::SoulseekRs::NotConnected)
        }
        fn get_all_downloads(&self) -> Vec<soulseek_rs::types::Download> {
            Vec::new()
        }
        fn pause_download(&self, _username: &str, _filename: &str) -> bool {
            false
        }
        fn resume_download(&self, _username: &str, _filename: &str) -> bool {
            false
        }
        fn remove_queued_download(
            &self,
            _username: &str,
            _filename: &str,
        ) -> bool {
            false
        }
        fn remove_download(&self, _username: &str, _filename: &str) -> bool {
            false
        }
        fn uploads(&self) -> Vec<soulseek_rs::UploadInfo> {
            Vec::new()
        }
        fn take_upload_events(&self) -> Vec<soulseek_rs::UploadInfo> {
            Vec::new()
        }
        fn cancel_upload(&self, _username: &str, _filename: &str) -> bool {
            false
        }
        fn set_upload_slots(&self, _slots: usize) {}
        fn check_privileges(&self) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn own_privilege_seconds(&self) -> Option<u32> {
            None
        }
        fn request_room_list(&self) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn join_room(&self, _room: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn leave_room(&self, _room: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn say_in_room(
            &self,
            _room: &str,
            _message: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn room_members(&self, _room: &str) -> Vec<String> {
            Vec::new()
        }
        fn take_room_events(&self) -> Vec<soulseek_rs::RoomEvent> {
            Vec::new()
        }
        fn send_private_message(
            &self,
            _username: &str,
            _message: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn take_private_messages(&self) -> Vec<soulseek_rs::UserMessage> {
            Vec::new()
        }
        fn browse_user(&self, _username: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn take_browse_result(
            &self,
            _username: &str,
        ) -> Option<Vec<soulseek_rs::SharedDirectory>> {
            None
        }
        fn request_user_info(&self, _username: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn user_info(&self, _username: &str) -> Option<soulseek_rs::UserInfo> {
            None
        }
        fn shared_counts(&self) -> (u32, u32) {
            (0, 0)
        }
        fn shared_directories(&self) -> Vec<String> {
            Vec::new()
        }
        fn set_shared_directories(
            &self,
            _directories: Vec<String>,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
    }
}
