//! Talking to a daemon as if it were a local client.
//!
//! [`RemoteSession`] implements the same [`SessionApi`] the commands and the
//! TUI already use, so nothing above the seam knows whether the session is in
//! this process or another one. Request/response methods are RPCs; the
//! destructive `take_*` drains read from a mirror the daemon feeds by pushing.

mod mirror;

use crate::api::{SessionApi, SessionSearch};
use crate::daemon::proto::{
    Ack, AuthParams, AuthResult, CODE_APPLICATION, ChatMessageDto,
    DaemonStatus, DirectoriesParams, DirectoryParams, DownloadDto,
    DownloadStartParams, DownloadStarted, Downloads, Event, IntervalSeconds,
    Members, MessageParams, Messages, Method, PROTOCOL_VERSION, QueryParams,
    Request, Response, RoomRef, RpcError, SayParams, SearchResults, Searches,
    Seconds, SharesStatus, SlotsParams, TransferRef, Uploads, UserRef,
    UserResult,
};
use mirror::Mirror;
use serde::de::DeserializeOwned;
use soulseek_rs::types::{Download, DownloadMetadata};
use soulseek_rs::{
    DownloadStatus, Result, RoomEvent, SearchResult, SessionLoss,
    SharedDirectory, SoulseekRs, UploadInfo, UserInfo, UserMessage,
};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long to wait for the daemon to answer one request. Every method is
/// either a lock-and-read or a message onto an already-open socket, so a
/// daemon that has not answered by now is wedged, not busy.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Where a daemon is listening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix socket path: local, and authenticated by its permissions.
    Socket(PathBuf),
    /// `host:port`: needs the token.
    Tcp(String),
}

impl Endpoint {
    /// Read what the user wrote. The distinction is unambiguous because a
    /// socket path always has a separator and `host:port` never does.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        if value.contains(std::path::MAIN_SEPARATOR) || value.contains('/') {
            return Self::Socket(PathBuf::from(value));
        }
        Self::Tcp(value.to_string())
    }

    /// Whether the transport itself proves who we are, in which case no token
    /// has to be found.
    const fn self_authenticating(&self) -> bool {
        matches!(self, Self::Socket(_))
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Socket(path) => write!(f, "{}", path.display()),
            Self::Tcp(address) => f.write_str(address),
        }
    }
}

/// Calls awaiting a reply, keyed by JSON-RPC id.
type Answer = std::result::Result<serde_json::Value, RpcError>;

#[derive(Default)]
struct Pending(Mutex<HashMap<u64, Sender<Answer>>>);

impl Pending {
    fn register(&self, id: u64) -> Receiver<Answer> {
        let (sender, receiver) = channel();
        if let Ok(mut pending) = self.0.lock() {
            pending.insert(id, sender);
        }
        receiver
    }

    fn settle(&self, id: u64, outcome: Answer) {
        let sender = self.0.lock().ok().and_then(|mut p| p.remove(&id));
        if let Some(sender) = sender {
            let _ = sender.send(outcome);
        }
    }

    fn abandon(&self, id: u64) {
        if let Ok(mut pending) = self.0.lock() {
            pending.remove(&id);
        }
    }

    /// Fail everything still waiting. Called when the reader stops, so callers
    /// blocked on a dead connection are told rather than left to time out.
    fn fail_all(&self) {
        let Ok(mut pending) = self.0.lock() else {
            return;
        };
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(RpcError::new(
                CODE_APPLICATION,
                "the daemon closed the connection",
            )));
        }
    }
}

/// The half of a connection that can be closed from another thread.
///
/// Dropping the write half is not enough to end the reader: it is blocked in
/// `read` on its own cloned handle, so the socket would stay open and the
/// daemon would keep a connection — and a hub subscription — for a client that
/// has gone. An explicit shutdown is what makes a dropped session actually
/// disconnect.
enum Closer {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Closer {
    fn close(&self) {
        let _ = match self {
            Self::Tcp(stream) => stream.shutdown(std::net::Shutdown::Both),
            #[cfg(unix)]
            Self::Unix(stream) => stream.shutdown(std::net::Shutdown::Both),
        };
    }
}

/// One connection to a daemon, presented as a session.
pub struct RemoteSession {
    outgoing: Mutex<Box<dyn Write + Send>>,
    closer: Closer,
    pending: Arc<Pending>,
    mirror: Arc<Mirror>,
    next_id: AtomicU64,
    username: String,
    /// Where this session was reached, so the UI can say so.
    endpoint: String,
    /// Set when the reader thread stops, so a call fails immediately rather
    /// than waiting out its timeout against a socket nobody is reading.
    closed: Arc<AtomicBool>,
}

impl RemoteSession {
    /// Connect and authenticate.
    ///
    /// `token` is required on TCP and optional on a Unix socket, where the
    /// socket's permissions have already answered the question.
    pub fn connect(endpoint: &Endpoint, token: Option<String>) -> Result<Self> {
        let (read, write, closer) = open(endpoint)?;

        let pending = Arc::new(Pending::default());
        let mirror = Arc::new(Mirror::default());
        let hung_up = Arc::new(AtomicBool::new(false));
        {
            let pending = Arc::clone(&pending);
            let mirror = Arc::clone(&mirror);
            let hung_up = Arc::clone(&hung_up);
            std::thread::spawn(move || {
                read_loop(read, &pending, &mirror);
                hung_up.store(true, Ordering::Relaxed);
                // Everything waiting on this connection has to be told, not
                // left to discover it by timing out: calls in flight, and
                // transfers whose progress only ever arrived as events.
                pending.fail_all();
                mirror.abandon_downloads();
            });
        }

        let mut session = Self {
            outgoing: Mutex::new(write),
            closer,
            pending,
            mirror,
            next_id: AtomicU64::new(1),
            username: String::new(),
            endpoint: endpoint.to_string(),
            closed: hung_up,
        };

        let token = if endpoint.self_authenticating() {
            // Offering a token here can only hurt: the socket's permissions
            // have already answered the question, and a stale one on disk
            // would turn a connection that should succeed into a refusal.
            None
        } else {
            Some(token.ok_or_else(|| {
                network(
                    "a remote daemon needs a token; see `soulseek-rs daemon \
                     token`",
                )
            })?)
        };
        let authenticated: AuthResult = session.request(
            Method::Auth,
            AuthParams {
                token,
                protocol: PROTOCOL_VERSION,
            },
        )?;
        if authenticated.protocol != PROTOCOL_VERSION {
            return Err(network(format!(
                "the daemon speaks protocol {} and this client speaks \
                 {PROTOCOL_VERSION}; upgrade whichever is older",
                authenticated.protocol
            )));
        }

        session.username = authenticated.username;
        Ok(session)
    }

    /// Send a request and wait for its reply.
    fn request<P: serde::Serialize, R: DeserializeOwned>(
        &self,
        method: Method,
        params: P,
    ) -> Result<R> {
        let value = self.call(method, params)?;
        serde_json::from_value(value).map_err(|e| {
            network(format!(
                "the daemon's answer to {method} was unreadable: {e}"
            ))
        })
    }

    fn call<P: serde::Serialize>(
        &self,
        method: Method,
        params: P,
    ) -> Result<serde_json::Value> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(SoulseekRs::ConnectionClosed);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(id)),
            method: method.wire_name().to_string(),
            params: serde_json::to_value(params)
                .map_err(|e| network(format!("cannot encode {method}: {e}")))?,
        };
        let line = serde_json::to_string(&request)
            .map_err(|e| network(format!("cannot encode {method}: {e}")))?;

        let answer = self.pending.register(id);
        {
            let mut outgoing =
                self.outgoing.lock().map_err(|_| SoulseekRs::LockPoisoned)?;
            if writeln!(outgoing, "{line}").is_err()
                || outgoing.flush().is_err()
            {
                self.pending.abandon(id);
                return Err(SoulseekRs::ConnectionClosed);
            }
        }

        match answer.recv_timeout(CALL_TIMEOUT) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(network(error.message)),
            Err(_) => {
                self.pending.abandon(id);
                Err(SoulseekRs::Timeout)
            }
        }
    }

    /// A call whose reply carries nothing worth reading.
    fn tell<P: serde::Serialize>(
        &self,
        method: Method,
        params: P,
    ) -> Result<()> {
        self.call(method, params).map(|_| ())
    }

    /// A call whose reply is a yes or a no. A daemon that cannot answer is a
    /// "no": every caller of these already reads false as "nothing to do".
    fn ask<P: serde::Serialize>(&self, method: Method, params: P) -> bool {
        self.request::<P, Ack>(method, params)
            .is_ok_and(|ack| ack.ok)
    }

    fn status(&self) -> Option<DaemonStatus> {
        self.request(Method::DaemonStatus, ()).ok()
    }

    fn shares(&self) -> SharesStatus {
        self.request(Method::SharesStatusOf, ())
            .unwrap_or(SharesStatus {
                folders: 0,
                files: 0,
                directories: Vec::new(),
            })
    }

    fn transfer(username: &str, filename: &str) -> TransferRef {
        TransferRef {
            username: username.to_string(),
            filename: filename.to_string(),
        }
    }

    /// Queue a transfer and hand back a channel that behaves like the library's.
    ///
    /// The daemon owns the real transfer; this mints the channel the caller
    /// expects and registers it with the mirror, so pushed updates arrive on it
    /// exactly as a local client's would.
    fn start_download(
        &self,
        filename: String,
        username: String,
        size: u64,
        metadata: DownloadMetadata,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        let (sender, receiver) = channel();
        // Register before asking: a transfer that fails instantly would
        // otherwise report into a mirror that is not listening yet.
        self.mirror
            .expect_download(&username, &filename, sender.clone());

        let started: DownloadStarted = self
            .request(
                Method::DownloadStart,
                DownloadStartParams {
                    username: username.clone(),
                    filename: filename.clone(),
                    size,
                    metadata: (&metadata).into(),
                },
            )
            .inspect_err(|_| {
                self.mirror.forget_download(&username, &filename);
            })?;

        Ok((rebuild(started.download, sender), receiver))
    }

    /// One parameterless request, for the control commands that only need to
    /// ask a daemon something once.
    pub fn ask_for<R: DeserializeOwned>(&self, method: Method) -> Result<R> {
        self.request(method, ())
    }
}

/// Turn the daemon's view of a transfer back into the library's, with the
/// caller's own channel spliced in.
fn rebuild(dto: DownloadDto, sender: Sender<DownloadStatus>) -> Download {
    let metadata = dto.metadata.into();
    Download {
        username: dto.username,
        filename: dto.filename,
        token: dto.token,
        size: dto.size,
        download_directory: dto.download_directory,
        status: dto.status.into(),
        sender,
        queue_position: dto.queue_position,
        metadata,
    }
}

fn network(message: impl Into<String>) -> SoulseekRs {
    SoulseekRs::InvalidMessage(message.into())
}

type Halves = (Box<dyn std::io::Read + Send>, Box<dyn Write + Send>, Closer);

fn open(endpoint: &Endpoint) -> Result<Halves> {
    match endpoint {
        Endpoint::Tcp(address) => {
            let stream = TcpStream::connect(address).map_err(|e| {
                network(format!("cannot reach the daemon at {address}: {e}"))
            })?;
            let read = stream.try_clone().map_err(SoulseekRs::NetworkError)?;
            let closer = Closer::Tcp(
                stream.try_clone().map_err(SoulseekRs::NetworkError)?,
            );
            Ok((Box::new(read), Box::new(stream), closer))
        }
        Endpoint::Socket(path) => connect_socket(path),
    }
}

#[cfg(unix)]
fn connect_socket(path: &Path) -> Result<Halves> {
    let stream =
        std::os::unix::net::UnixStream::connect(path).map_err(|e| {
            network(format!(
                "cannot reach the daemon at {}: {e}",
                path.display()
            ))
        })?;
    let read = stream.try_clone().map_err(SoulseekRs::NetworkError)?;
    let closer =
        Closer::Unix(stream.try_clone().map_err(SoulseekRs::NetworkError)?);
    Ok((Box::new(read), Box::new(stream), closer))
}

#[cfg(not(unix))]
fn connect_socket(path: &Path) -> Result<Halves> {
    Err(network(format!(
        "this platform has no Unix sockets, so {} cannot be a daemon address; \
         use host:port",
        path.display()
    )))
}

impl Drop for RemoteSession {
    fn drop(&mut self) {
        // Tell the daemon we are gone, so it can release the connection and
        // stop fanning events at a client that is not there.
        self.closer.close();
    }
}

/// Route everything the daemon sends: replies to the call waiting for them,
/// events to the mirror.
fn read_loop(
    read: Box<dyn std::io::Read + Send>,
    pending: &Pending,
    mirror: &Mirror,
) {
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else {
            return;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(response) = serde_json::from_str::<Response>(&line) else {
            soulseek_rs::warn!("ignoring an unreadable line from the daemon");
            continue;
        };

        if let Some(method) = response.method.as_deref() {
            if let Some(event) = Event::from_wire(method) {
                mirror.apply(event, response.params.unwrap_or_default());
            }
            continue;
        }

        let Some(id) = response.id.as_ref().and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        match (response.result, response.error) {
            (Some(result), _) => pending.settle(id, Ok(result)),
            (None, Some(error)) => pending.settle(id, Err(error)),
            (None, None) => pending.settle(id, Ok(serde_json::Value::Null)),
        }
    }
}

impl SessionApi for RemoteSession {
    fn username(&self) -> String {
        self.username.clone()
    }

    fn daemon_endpoint(&self) -> Option<String> {
        Some(self.endpoint.clone())
    }

    fn listen_port(&self) -> Option<u16> {
        self.status().and_then(|status| status.listen_port)
    }

    fn session_loss(&self) -> Option<SessionLoss> {
        // The pushed event is the prompt answer; the status call also covers a
        // loss that happened before this client attached.
        self.mirror.loss().or_else(|| {
            self.status()
                .and_then(|status| status.session_loss)
                .map(Into::into)
        })
    }

    fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>> {
        self.tell(
            Method::SearchStart,
            QueryParams {
                query: query.to_string(),
            },
        )?;
        // The window is spent here rather than in the daemon: one client's
        // search must not stop the daemon answering everybody else.
        soulseek_rs::Client::collect_for(timeout, cancel);
        Ok(self.get_search_results(query))
    }

    fn get_search_results(&self, key: &str) -> Vec<SearchResult> {
        self.request::<_, SearchResults>(
            Method::SearchResults,
            QueryParams {
                query: key.to_string(),
            },
        )
        .map(|found| found.results.into_iter().map(Into::into).collect())
        .unwrap_or_default()
    }

    fn all_searches(&self) -> Vec<SessionSearch> {
        self.request::<_, Searches>(Method::SearchList, ())
            .map(|listed| {
                listed
                    .searches
                    .into_iter()
                    .map(|search| SessionSearch {
                        query: search.query,
                        files: search.files,
                        started_secs_ago: Some(search.started_secs_ago),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn forget_search(&self, query: &str) -> bool {
        self.ask(
            Method::SearchForget,
            QueryParams {
                query: query.to_string(),
            },
        )
    }

    fn get_search_results_count(&self, key: &str) -> usize {
        self.get_search_results(key).len()
    }

    fn try_get_search_results(&self, key: &str) -> Option<Vec<SearchResult>> {
        Some(self.get_search_results(key))
    }

    fn start_wishlist_search(&self, query: &str) -> Result<()> {
        self.tell(
            Method::SearchWishlist,
            QueryParams {
                query: query.to_string(),
            },
        )
    }

    fn wishlist_interval(&self) -> Duration {
        self.request::<_, IntervalSeconds>(Method::SearchWishlistInterval, ())
            .map_or(Duration::from_mins(12), |interval| {
                Duration::from_secs(interval.seconds)
            })
    }

    fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        _download_directory: String,
        // the daemon's own download directory wins — a remote caller
        // cannot see the filesystem the bytes land on. Plumb a per-transfer
        // directory through if a client ever needs to sort into subfolders.
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        self.start_download(
            filename,
            username,
            size,
            DownloadMetadata::default(),
        )
    }

    fn download_with_metadata(
        &self,
        filename: String,
        username: String,
        size: u64,
        _download_directory: String,
        metadata: DownloadMetadata,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        self.start_download(filename, username, size, metadata)
    }

    fn get_all_downloads(&self) -> Vec<Download> {
        let Ok(listed) = self.request::<_, Downloads>(Method::DownloadList, ())
        else {
            return Vec::new();
        };
        listed
            .downloads
            .into_iter()
            .map(|dto| {
                // Nothing reads this sender: a listing covers other clients'
                // transfers too, and the ones this client started already have
                // a live channel from `download`.
                let (sender, _) = channel();
                rebuild(dto, sender)
            })
            .collect()
    }

    fn pause_download(&self, username: &str, filename: &str) -> bool {
        self.ask(Method::DownloadPause, Self::transfer(username, filename))
    }

    fn resume_download(&self, username: &str, filename: &str) -> bool {
        self.ask(Method::DownloadResume, Self::transfer(username, filename))
    }

    fn remove_queued_download(&self, username: &str, filename: &str) -> bool {
        self.ask(
            Method::DownloadRemoveQueued,
            Self::transfer(username, filename),
        )
    }

    fn remove_download(&self, username: &str, filename: &str) -> bool {
        self.mirror.forget_download(username, filename);
        self.ask(Method::DownloadRemove, Self::transfer(username, filename))
    }

    fn uploads(&self) -> Vec<UploadInfo> {
        self.request::<_, Uploads>(Method::UploadList, ())
            .map(|listed| listed.uploads.into_iter().map(Into::into).collect())
            .unwrap_or_default()
    }

    fn take_upload_events(&self) -> Vec<UploadInfo> {
        self.mirror.take_uploads()
    }

    fn cancel_upload(&self, username: &str, filename: &str) -> bool {
        self.ask(Method::UploadCancel, Self::transfer(username, filename))
    }

    fn set_upload_slots(&self, slots: usize) {
        let _ = self.tell(Method::UploadSlots, SlotsParams { slots });
    }

    fn check_privileges(&self) -> Result<()> {
        self.tell(Method::PrivilegesCheck, ())
    }

    fn own_privilege_seconds(&self) -> Option<u32> {
        self.request::<_, Seconds>(Method::PrivilegesOwn, ())
            .ok()?
            .seconds
    }

    fn request_room_list(&self) -> Result<()> {
        self.tell(Method::RoomListRequest, ())
    }

    fn join_room(&self, room: &str) -> Result<()> {
        self.tell(
            Method::RoomJoin,
            RoomRef {
                room: room.to_string(),
            },
        )
    }

    fn leave_room(&self, room: &str) -> Result<()> {
        self.tell(
            Method::RoomLeave,
            RoomRef {
                room: room.to_string(),
            },
        )
    }

    fn say_in_room(&self, room: &str, message: &str) -> Result<()> {
        self.tell(
            Method::RoomSay,
            SayParams {
                room: room.to_string(),
                message: message.to_string(),
            },
        )
    }

    fn room_members(&self, room: &str) -> Vec<String> {
        self.request::<_, Members>(
            Method::RoomMembers,
            RoomRef {
                room: room.to_string(),
            },
        )
        .map(|members| members.users)
        .unwrap_or_default()
    }

    fn take_room_events(&self) -> Vec<RoomEvent> {
        self.mirror.take_rooms()
    }

    fn send_private_message(
        &self,
        username: &str,
        message: &str,
    ) -> Result<()> {
        self.tell(
            Method::MessageSend,
            MessageParams {
                username: username.to_string(),
                message: message.to_string(),
            },
        )
    }

    fn take_private_messages(&self) -> Vec<UserMessage> {
        self.mirror.take_messages()
    }

    fn browse_user(&self, username: &str) -> Result<()> {
        self.tell(
            Method::BrowseUser,
            UserRef {
                username: username.to_string(),
            },
        )
    }

    fn take_browse_result(
        &self,
        username: &str,
    ) -> Option<Vec<SharedDirectory>> {
        self.mirror.take_browse(username)
    }

    fn request_user_info(&self, username: &str) -> Result<()> {
        self.tell(
            Method::UserRequest,
            UserRef {
                username: username.to_string(),
            },
        )
    }

    fn user_info(&self, username: &str) -> Option<UserInfo> {
        self.request::<_, UserResult>(
            Method::UserInfoOf,
            UserRef {
                username: username.to_string(),
            },
        )
        .ok()?
        .user
        .map(Into::into)
    }

    fn message_history(&self) -> Vec<ChatMessageDto> {
        self.request::<_, Messages>(Method::MessageHistory, ())
            .map(|history| history.messages)
            .unwrap_or_default()
    }

    fn shared_counts(&self) -> (u32, u32) {
        let shares = self.shares();
        (shares.folders, shares.files)
    }

    fn shared_directories(&self) -> Vec<String> {
        self.shares().directories
    }

    fn set_shared_directories(&self, directories: Vec<String>) -> Result<()> {
        self.tell(Method::SharesSet, DirectoriesParams { directories })
    }

    fn set_download_directory(&self, directory: String) -> Result<()> {
        self.tell(Method::DownloadSetDir, DirectoryParams { directory })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_tells_a_socket_path_from_a_host_and_port() {
        assert_eq!(
            Endpoint::parse("/run/soulseek/daemon.sock"),
            Endpoint::Socket(PathBuf::from("/run/soulseek/daemon.sock"))
        );
        assert_eq!(
            Endpoint::parse("127.0.0.1:5030"),
            Endpoint::Tcp("127.0.0.1:5030".to_string())
        );
        assert_eq!(
            Endpoint::parse("daemon.example:5030"),
            Endpoint::Tcp("daemon.example:5030".to_string())
        );
    }

    #[test]
    fn only_a_socket_authenticates_itself() {
        assert!(Endpoint::parse("/tmp/daemon.sock").self_authenticating());
        assert!(!Endpoint::parse("host:5030").self_authenticating());
    }

    #[test]
    fn a_reply_settles_the_call_that_is_waiting_for_it() {
        let pending = Pending::default();
        let answer = pending.register(7);
        pending.settle(7, Ok(serde_json::json!({ "ok": true })));
        assert_eq!(
            answer.recv().unwrap().unwrap(),
            serde_json::json!({ "ok": true })
        );
    }

    #[test]
    fn a_reply_for_an_abandoned_call_is_dropped_quietly() {
        let pending = Pending::default();
        let answer = pending.register(7);
        pending.abandon(7);
        pending.settle(7, Ok(serde_json::Value::Null));
        assert!(answer.recv().is_err());
    }

    #[test]
    fn a_closed_connection_fails_every_waiting_call() {
        // Otherwise each blocked caller would sit out the full timeout against
        // a socket nobody is reading.
        let pending = Pending::default();
        let first = pending.register(1);
        let second = pending.register(2);
        pending.fail_all();
        assert!(first.recv().unwrap().is_err());
        assert!(second.recv().unwrap().is_err());
    }

    #[test]
    fn the_reader_routes_replies_to_calls_and_events_to_the_mirror() {
        let pending = Pending::default();
        let mirror = Mirror::default();
        let answer = pending.register(1);

        let lines = concat!(
            r#"{"jsonrpc":"2.0","method":"event.room","params":{"event":"left","room":"lobby"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
            "\n",
        );
        read_loop(Box::new(lines.as_bytes()), &pending, &mirror);

        assert_eq!(
            answer.recv().unwrap().unwrap(),
            serde_json::json!({ "ok": true })
        );
        assert_eq!(
            mirror.take_rooms(),
            [RoomEvent::Left {
                room: "lobby".into()
            }]
        );
    }

    #[test]
    fn an_error_reply_reaches_the_caller_as_an_error() {
        let pending = Pending::default();
        let mirror = Mirror::default();
        let answer = pending.register(1);

        read_loop(
            Box::new(
                concat!(
                    r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"nope","data":{"exit":4}}}"#,
                    "\n"
                )
                .as_bytes(),
            ),
            &pending,
            &mirror,
        );

        let error = answer.recv().unwrap().expect_err("an error reply");
        assert_eq!(error.message, "nope");
        assert_eq!(error.exit(), crate::output::Exit::NoResults);
    }

    #[test]
    fn a_junk_line_does_not_stop_the_reader() {
        let pending = Pending::default();
        let mirror = Mirror::default();
        let answer = pending.register(1);

        read_loop(
            Box::new(
                concat!(
                    "not json at all\n",
                    r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
                    "\n"
                )
                .as_bytes(),
            ),
            &pending,
            &mirror,
        );

        assert!(
            answer.recv().unwrap().is_ok(),
            "one bad line must not cost the reply behind it"
        );
    }
}
