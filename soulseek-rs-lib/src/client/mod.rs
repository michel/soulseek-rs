use crate::actor::ActorHandle;
use crate::actor::server_actor::{
    PeerAddress, ServerActor, ServerMessage, UserMessage,
};
use crate::download_store::{DownloadStore, collect_failed_tokens};
use crate::types::{DownloadMetadata, DownloadStatus, RoomEvent, RoomInfo};
use crate::utils::logger;
use crate::{
    Transfer,
    actor::{ActorSystem, peer_registry::PeerRegistry},
    error::{Result, SoulseekRs},
    message::peer::{FileEntry, SharedDirectory, build_file_search_response},
    peer::{
        ConnectionType, DownloadPeer, NewPeer, Peer, PeerMessage,
        listen::Listen,
    },
    shares::Shares,
    types::{Download, Search, SearchResult},
    utils::{lock::RwLockExt, md5, thread_pool::ThreadPool},
};
use std::{
    collections::HashMap,
    net::TcpStream,
    sync::{
        RwLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, Sender},
    },
    thread::{self, sleep},
};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace, warn};
const DEFAULT_LISTEN_PORT: u16 = 2234;

/// How long to wait for a server-brokered (firewalled) peer to connect back
/// before giving up and failing the download. Matches the direct-dial timeout.
const BROKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Source of non-zero correlation tokens for server-brokered connections.
static NEXT_CONNECT_TOKEN: AtomicU32 = AtomicU32::new(1);

fn next_connect_token() -> u32 {
    NEXT_CONNECT_TOKEN.fetch_add(1, Ordering::Relaxed).max(1)
}

/// Upload tokens are minted in the high half of the space so they never collide
/// with download tokens (md5-derived, always < 2^20).
static NEXT_UPLOAD_TOKEN: AtomicU32 = AtomicU32::new(0x8000_0000);

fn next_upload_token() -> u32 {
    NEXT_UPLOAD_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// A file we have agreed to serve to a peer, awaiting their TransferResponse.
struct UploadJob {
    downloader: String,
    real_path: std::path::PathBuf,
    virtual_path: String,
    size: u64,
}

/// Live bookkeeping for an upload being served (or recently finished).
struct ActiveUpload {
    username: String,
    filename: String,
    size: u64,
    bytes_sent: Arc<std::sync::atomic::AtomicU64>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    status: crate::types::UploadStatus,
}

/// Build a `FileSearchResponse` for `query` against `shares`, or `None` if
/// nothing matches. `own_username` is the name the searcher will download from.
fn build_search_response(
    shares: &Shares,
    own_username: &str,
    token: u32,
    query: &str,
) -> Option<crate::message::Message> {
    let matches = shares.search(query);
    if matches.is_empty() {
        return None;
    }
    let entries: Vec<FileEntry> = matches
        .iter()
        .map(|f| FileEntry {
            name: &f.virtual_path,
            size: f.size,
            attribs: &f.attributes,
        })
        .collect();
    Some(build_file_search_response(
        own_username,
        token,
        &entries,
        1,
        0,
    ))
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub username: String,
    pub password: String,
    pub server_address: PeerAddress,
    pub enable_listen: bool,
    pub listen_port: u16,
    /// Directories whose files are shared with (uploaded to) other peers.
    /// Empty means nothing is shared.
    pub shared_directories: Vec<String>,
}

impl ClientSettings {
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            ..Default::default()
        }
    }
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            server_address: PeerAddress::new(
                "server.slsknet.org".to_string(),
                2416,
            ),
            enable_listen: true,
            listen_port: DEFAULT_LISTEN_PORT,
            shared_directories: Vec::new(),
        }
    }
}

#[derive(Debug)]

pub enum ClientOperation {
    NewPeer(NewPeer),
    ConnectToPeer(Peer),
    SearchResult(SearchResult),
    PeerDisconnected(u64, String, Option<SoulseekRs>),
    PierceFireWall(Peer),
    DownloadFromPeer(u32, Peer, bool),
    UpdateDownloadTokens(Transfer, String),
    GetPeerAddressResponse {
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    },
    UploadFailed(String, String),
    PlaceInQueueUpdate {
        username: String,
        filename: String,
        place: u32,
    },
    SetServerSender(Sender<ServerMessage>),
    PrivateMessageReceived(UserMessage),
    PeerConnected(String),
    /// A search distributed to us by the server; reply if our shares match.
    IncomingSearch {
        username: String,
        token: u32,
        query: String,
    },
    /// A peer queued one of our shared files; `requester_key` is the registry
    /// key of the peer actor (may carry a `:direct` suffix).
    QueueUpload {
        requester_key: String,
        filename: String,
    },
    /// The peer accepted our upload offer for `token`; start streaming.
    StartUpload {
        token: u32,
    },
    /// A peer asked to browse our shared files; send our SharedFileListResponse.
    ShareListRequested {
        requester_key: String,
    },
    /// A peer we are browsing returned their shared-file listing.
    BrowseResult {
        username: String,
        directories: Vec<SharedDirectory>,
    },
    /// A direct outbound connection to this peer failed before it was
    /// established — the peer is likely firewalled, so fall back to asking the
    /// server to broker the connection. Carries the reporting actor's id.
    PeerConnectFailed(u64, String),
    /// Something happened in the chat-room subsystem (list refreshed, a room
    /// joined/left, a message said, a member joined/left).
    RoomEvent(RoomEvent),
}
pub struct ClientContext {
    pub peer_registry: Option<PeerRegistry>,
    pub downloads: DownloadStore,
    sender: Option<Sender<ClientOperation>>,
    server_sender: Option<Sender<ServerMessage>>,
    searches: HashMap<String, Search>,
    private_messages: Vec<UserMessage>,
    /// Correlation tokens for server-brokered (firewalled) connections, mapping
    /// a token we sent in a ConnectToPeer to the peer we expect back.
    pending_connect_tokens: HashMap<u32, String>,
    /// Files we share with peers (read-only after connect).
    pub shares: Arc<Shares>,
    /// The directories the current share index was built from.
    pub shared_directories: Vec<String>,
    /// Peer listen addresses learned from GetPeerAddress responses.
    peer_addresses: HashMap<String, (String, u32)>,
    /// Peer messages waiting for a control connection to that peer.
    pending_peer_messages: HashMap<String, Vec<crate::message::Message>>,
    /// Uploads we have offered, keyed by our transfer token.
    uploads: HashMap<u32, UploadJob>,
    active_uploads: HashMap<u32, ActiveUpload>,
    /// Upload tokens waiting for the downloader's address to be resolved.
    pending_serves: HashMap<String, Vec<u32>>,
    /// Shared-file listings received from peers we browsed.
    browse_results: HashMap<String, Vec<SharedDirectory>>,
    /// Latest snapshot of the public chat-room list (from `RoomList`, code 64).
    room_list: Vec<RoomInfo>,
    /// Chat-room events awaiting consumption by the client/UI.
    room_events: Vec<RoomEvent>,
    actor_system: Arc<ActorSystem>,
}
impl Default for ClientContext {
    fn default() -> Self {
        Self::new()
    }
}

// Thin delegating shims so existing callers (peer/listen, peer/download_peer,
// tests) keep working while download state lives in DownloadStore.
impl ClientContext {
    pub fn add_download(&mut self, download: Download) {
        self.downloads.add(download);
    }
    pub fn remove_download(&mut self, token: u32) {
        self.downloads.remove(token);
    }
    #[must_use]
    pub fn get_download_by_token(&self, token: u32) -> Option<&Download> {
        self.downloads.get_by_token(token)
    }
    pub fn get_download_by_token_mut(
        &mut self,
        token: u32,
    ) -> Option<&mut Download> {
        self.downloads.get_by_token_mut(token)
    }
    pub fn get_download_by_file_mut(
        &mut self,
        username: &str,
        filename: &str,
    ) -> Option<&mut Download> {
        self.downloads.get_by_file_mut(username, filename)
    }
    #[must_use]
    pub fn get_download_tokens(&self) -> Vec<u32> {
        self.downloads.tokens()
    }
    #[must_use]
    pub const fn get_downloads(&self) -> &Vec<Download> {
        self.downloads.list()
    }
    pub fn update_download_with_status(
        &mut self,
        token: u32,
        status: DownloadStatus,
    ) {
        self.downloads.update_status(token, status);
    }
    pub fn remove_queued_download_by_file(
        &mut self,
        username: &str,
        filename: &str,
    ) -> bool {
        self.downloads.remove_queued_by_file(username, filename)
    }
}
#[test]
fn test_client_context_downloads() {
    let mut context = ClientContext::new();
    let token = 123;
    let new_token = 1234;
    let download = Download {
        username: "test".to_string(),
        filename: "test.txt".to_string(),
        token,
        size: 100,
        download_directory: "test".to_string(),
        status: DownloadStatus::Queued,
        sender: mpsc::channel().0,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    };
    context.add_download(download);
    assert!(context.get_download_by_token(123).is_some());
    assert_eq!(context.get_download_tokens(), vec![123]);
    assert_eq!(context.get_downloads().len(), 1);
    if let Some(download) = context.get_download_by_token_mut(token) {
        assert_eq!(download.token, token);
        download.token = new_token;
    }
    assert!(context.get_download_by_token(new_token).is_some());
    assert_eq!(context.get_download_tokens(), vec![new_token]);
    context.remove_download(new_token);
    assert_eq!(context.get_downloads().len(), 0);
    assert!(context.get_download_by_token(1234).is_none());
}

#[test]
fn test_client_pause_and_resume_download() {
    let client = Client::new("test-user", "test-password");
    let (download_sender, download_receiver) = mpsc::channel();
    let download = Download {
        username: "peer".to_string(),
        filename: "song.mp3".to_string(),
        token: 123,
        size: 100,
        download_directory: "test".to_string(),
        status: DownloadStatus::InProgress {
            bytes_downloaded: 25,
            total_bytes: 100,
            speed_bytes_per_sec: 10.0,
        },
        sender: download_sender,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    };

    client.context.write().unwrap().add_download(download);

    assert!(client.pause_download("peer", "song.mp3"));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(123)
            .unwrap()
            .status,
        DownloadStatus::Paused {
            bytes_downloaded: 25,
            total_bytes: 100
        }
    ));
    assert!(matches!(
        download_receiver.try_recv().unwrap(),
        DownloadStatus::Paused {
            bytes_downloaded: 25,
            total_bytes: 100
        }
    ));

    assert!(client.resume_download("peer", "song.mp3"));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(123)
            .unwrap()
            .status,
        DownloadStatus::InProgress {
            bytes_downloaded: 25,
            total_bytes: 100,
            speed_bytes_per_sec: 0.0
        }
    ));
}

#[test]
fn download_without_a_connection_resolves_failed() {
    // A client that never connected has no server handle and no peer registry,
    // so it cannot open a connection to the peer: the download must resolve to
    // Failed rather than hang Queued forever.
    let client = Client::new("test-user", "test-password");
    let (_download, receiver) = client
        .download(
            "song.mp3".to_string(),
            "peer".to_string(),
            100,
            "test".to_string(),
        )
        .expect("download() should return a handle");
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(1)),
        Ok(DownloadStatus::Failed(_))
    ));
}

#[test]
fn fail_queued_downloads_notifies_receiver_and_store() {
    // When a brokered connect times out, every Queued download for the peer
    // must resolve to Failed both on its channel and in the store.
    let client = Client::new("u", "p");
    let (sender, receiver) = mpsc::channel();
    client.context.write().unwrap().add_download(Download {
        username: "peer".to_string(),
        filename: "f.mp3".to_string(),
        token: 7,
        size: 10,
        download_directory: "d".to_string(),
        status: DownloadStatus::Queued,
        sender,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    });

    Client::fail_queued_downloads(&client.context, "peer");

    assert!(matches!(receiver.try_recv(), Ok(DownloadStatus::Failed(_))));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(7)
            .unwrap()
            .status,
        DownloadStatus::Failed(_)
    ));
}

#[test]
fn build_search_response_matches_shares_and_echoes_token() {
    let dir = std::env::temp_dir()
        .join(format!("soulseek-searchresp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe_xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    let response = build_search_response(&shares, "me", 99, "xyzzy")
        .expect("a matching share yields a response");
    let mut decoded =
        crate::message::Message::new_with_data(response.get_buffer());
    decoded.set_pointer(8);
    let result = SearchResult::new_from_message(&mut decoded).unwrap();
    assert_eq!(result.username, "me");
    assert_eq!(result.token, 99);
    assert!(result.files.iter().any(|f| f.name.contains("probe_xyzzy")));

    assert!(build_search_response(&shares, "me", 1, "nomatch").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_client_removes_only_queued_downloads() {
    let client = Client::new("test-user", "test-password");
    let queued_download = Download {
        username: "peer".to_string(),
        filename: "queued.mp3".to_string(),
        token: 123,
        size: 100,
        download_directory: "test".to_string(),
        status: DownloadStatus::Queued,
        sender: mpsc::channel().0,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    };
    let active_download = Download {
        username: "peer".to_string(),
        filename: "active.mp3".to_string(),
        token: 456,
        size: 100,
        download_directory: "test".to_string(),
        status: DownloadStatus::InProgress {
            bytes_downloaded: 25,
            total_bytes: 100,
            speed_bytes_per_sec: 10.0,
        },
        sender: mpsc::channel().0,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    };

    {
        let mut context = client.context.write().unwrap();
        context.add_download(queued_download);
        context.add_download(active_download);
    }

    assert!(client.remove_queued_download("peer", "queued.mp3"));
    assert!(!client.remove_queued_download("peer", "active.mp3"));
    let context = client.context.read().unwrap();
    assert!(context.get_download_by_token(123).is_none());
    assert!(context.get_download_by_token(456).is_some());
}

impl ClientContext {
    #[must_use]
    pub fn new() -> Self {
        let max_threads =
            thread::available_parallelism().map_or(8, std::num::NonZero::get);

        let thread_pool = Arc::new(ThreadPool::new(max_threads));
        let actor_system = Arc::new(ActorSystem::new(thread_pool));

        Self {
            peer_registry: None,
            sender: None,
            server_sender: None,
            searches: HashMap::new(),
            private_messages: Vec::new(),
            pending_connect_tokens: HashMap::new(),
            shares: Arc::new(Shares::empty()),
            shared_directories: Vec::new(),
            peer_addresses: HashMap::new(),
            pending_peer_messages: HashMap::new(),
            uploads: HashMap::new(),
            active_uploads: HashMap::new(),
            pending_serves: HashMap::new(),
            browse_results: HashMap::new(),
            room_list: Vec::new(),
            room_events: Vec::new(),
            downloads: DownloadStore::new(),
            actor_system,
        }
    }

    /// Apply a chat-room event: keep the room-list snapshot current and queue
    /// the event for the client/UI to drain.
    pub fn apply_room_event(&mut self, event: RoomEvent) {
        if let RoomEvent::List(rooms) = &event {
            self.room_list.clone_from(rooms);
        }
        self.room_events.push(event);
    }

    /// The latest snapshot of the public chat-room list.
    #[must_use]
    pub fn room_list(&self) -> Vec<RoomInfo> {
        self.room_list.clone()
    }

    /// Remove and return all chat-room events received since the last call.
    #[must_use]
    pub fn take_room_events(&mut self) -> Vec<RoomEvent> {
        std::mem::take(&mut self.room_events)
    }

    /// Cache a peer's listen address learned from a GetPeerAddress response.
    pub fn cache_peer_address(
        &mut self,
        username: &str,
        host: String,
        port: u32,
    ) {
        self.peer_addresses
            .insert(username.to_string(), (host, port));
    }

    /// The cached listen address for `username`, if known.
    #[must_use]
    pub fn peer_address(&self, username: &str) -> Option<(String, u32)> {
        self.peer_addresses.get(username).cloned()
    }

    /// Queue a peer message to send once a control connection to `username` is up.
    pub fn queue_peer_message(
        &mut self,
        username: &str,
        message: crate::message::Message,
    ) {
        self.pending_peer_messages
            .entry(username.to_string())
            .or_default()
            .push(message);
    }

    /// Remove and return the messages queued for `username`.
    pub fn take_peer_messages(
        &mut self,
        username: &str,
    ) -> Vec<crate::message::Message> {
        self.pending_peer_messages
            .remove(username)
            .unwrap_or_default()
    }

    /// Store a shared-file listing received from browsing `username`.
    pub fn store_browse_result(
        &mut self,
        username: String,
        directories: Vec<SharedDirectory>,
    ) {
        self.browse_results.insert(username, directories);
    }

    /// Remove and return the shared-file listing browsed from `username`.
    pub fn take_browse_result(
        &mut self,
        username: &str,
    ) -> Option<Vec<SharedDirectory>> {
        self.browse_results.remove(username)
    }

    /// Remember that a server-brokered connection to `username` is pending under
    /// `token`; the peer will quote it back in a PierceFirewall.
    pub fn add_pending_connect(&mut self, token: u32, username: String) {
        self.pending_connect_tokens.insert(token, username);
    }

    /// Resolve and consume the peer expected for a brokered connection `token`.
    pub fn take_pending_connect(&mut self, token: u32) -> Option<String> {
        self.pending_connect_tokens.remove(&token)
    }

    /// Record a private message received from another user.
    pub fn push_private_message(&mut self, message: UserMessage) {
        self.private_messages.push(message);
    }

    /// Remove and return all buffered private messages.
    pub fn take_private_messages(&mut self) -> Vec<UserMessage> {
        std::mem::take(&mut self.private_messages)
    }
}
pub struct Client {
    enable_listen: bool,
    listen_port: u16,
    address: PeerAddress,
    username: String,
    password: String,
    shared_directories: Vec<String>,
    server_handle: Option<ActorHandle<ServerMessage>>,
    context: Arc<RwLock<ClientContext>>,
}

impl Client {
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self::with_settings(ClientSettings::new(username, password))
    }

    #[must_use]
    pub fn with_settings(settings: ClientSettings) -> Self {
        logger::init();
        Self {
            enable_listen: settings.enable_listen,
            listen_port: settings.listen_port,
            address: settings.server_address,
            username: settings.username,
            password: settings.password,
            shared_directories: settings.shared_directories,
            context: Arc::new(RwLock::new(ClientContext::new())),
            server_handle: None,
        }
    }

    /// The directories whose files are currently shared with other peers.
    #[must_use]
    pub fn shared_directories(&self) -> Vec<String> {
        self.context
            .read_safe()
            .map(|ctx| ctx.shared_directories.clone())
            .unwrap_or_default()
    }

    /// Snapshot of the uploads served this session (active and finished),
    /// most recent last.
    #[must_use]
    pub fn uploads(&self) -> Vec<crate::types::UploadInfo> {
        self.context.read_safe().map_or_else(
            |_| Vec::new(),
            |ctx| {
                let mut tokens: Vec<&u32> = ctx.active_uploads.keys().collect();
                tokens.sort_unstable();
                tokens
                    .into_iter()
                    .map(|token| {
                        let upload = &ctx.active_uploads[token];
                        crate::types::UploadInfo {
                            username: upload.username.clone(),
                            filename: upload.filename.clone(),
                            size: upload.size,
                            bytes_sent: upload
                                .bytes_sent
                                .load(std::sync::atomic::Ordering::Relaxed),
                            status: upload.status.clone(),
                        }
                    })
                    .collect()
            },
        )
    }

    /// Ask an in-progress upload to `username` of `filename` to stop.
    /// Returns whether a matching in-progress upload was found.
    #[must_use = "returns whether a matching upload was found"]
    pub fn cancel_upload(&self, username: &str, filename: &str) -> bool {
        self.context.read_safe().is_ok_and(|ctx| {
            let mut found = false;
            for upload in ctx.active_uploads.values() {
                if upload.username == username
                    && upload.filename == filename
                    && upload.status == crate::types::UploadStatus::InProgress
                {
                    upload
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    found = true;
                }
            }
            found
        })
    }

    /// Replace the shared directories at runtime: rescan into a fresh
    /// index (served to peers from then on) and re-announce the new
    /// folder/file counts to the server.
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn set_shared_directories(&self, dirs: Vec<String>) -> Result<()> {
        let roots: Vec<std::path::PathBuf> = dirs
            .iter()
            .filter(|dir| !dir.trim().is_empty())
            .map(std::path::PathBuf::from)
            .collect();
        let shares = if roots.is_empty() {
            Shares::empty()
        } else {
            Shares::scan_many(&roots)
        };
        info!(
            "Now sharing {} files in {} folders from {} directories",
            shares.file_count(),
            shares.folder_count(),
            roots.len()
        );
        let folder_count = shares.folder_count();
        let file_count = shares.file_count();
        {
            let mut ctx = self.context.write_safe()?;
            ctx.shares = Arc::new(shares);
            ctx.shared_directories = dirs;
        }
        self.send_server_message(
            crate::message::server::MessageFactory::build_shared_folders_message(
                folder_count,
                file_count,
            ),
        )
    }
}

mod connection;
mod downloads;
mod operations;
mod rooms;
mod search;
mod uploads;
