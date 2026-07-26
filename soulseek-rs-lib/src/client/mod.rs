use crate::actor::ActorHandle;
use crate::actor::server_actor::{
    PeerAddress, ServerActor, ServerMessage, UserMessage,
};
use crate::download_store::{DownloadStore, collect_failed_tokens};
use crate::types::{
    ClientVersion, DownloadMetadata, DownloadStatus, RoomEvent, RoomInfo,
    SessionLoss, SessionWatch, UserInfo, UserPresence, UserStats, UserStatus,
};
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
    utils::{lock::RwLockExt, md5},
};
use std::{
    collections::{HashMap, HashSet},
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
use upload_queue::QueuedUpload;

use crate::{debug, error, info, trace, warn};
const DEFAULT_LISTEN_PORT: u16 = 2234;

/// What to assume between wishlist searches until the server announces its own.
///
/// Twelve minutes is what Soulseek servers give an unprivileged account (code
/// 104); guessing lower only gets the searches dropped.
pub const DEFAULT_WISHLIST_INTERVAL: Duration = Duration::from_mins(12);

/// How many uploads run at once by default.
///
/// The cap is what makes the queue real — with unlimited slots there is nothing
/// for a privileged peer to jump — but it must not be the thing throttling a
/// modern connection. On Soulseek the per-transfer rate is usually set by the
/// *other* end, so concurrency is what fills an uplink: ten peers at a typical
/// few hundred KiB/s each is a few MiB/s, which a 50 Mbit uplink carries and a
/// pair of slots leaves mostly idle. Ten still divides a modest uplink into
/// shares big enough that no peer times out waiting.
///
/// Measured on the stress benchmark (64 waiting peers, loopback): two slots
/// took 24.2s, eight took 6.2s, thirty-two took 3.1s.
pub const DEFAULT_UPLOAD_SLOTS: usize = 10;

/// How long to wait for a server-brokered (firewalled) peer to connect back
/// before giving up and failing the download. Matches the direct-dial timeout.
const BROKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Source of non-zero correlation tokens for server-brokered connections.
static NEXT_CONNECT_TOKEN: AtomicU32 = AtomicU32::new(1);

fn next_connect_token() -> u32 {
    NEXT_CONNECT_TOKEN.fetch_add(1, Ordering::Relaxed).max(1)
}

/// Upload tokens are minted in the high half of the space so they never collide
/// with download tokens, which are always < 2^31.
static NEXT_UPLOAD_TOKEN: AtomicU32 = AtomicU32::new(0x8000_0000);

fn next_upload_token() -> u32 {
    NEXT_UPLOAD_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// Source of download tokens, kept in the low half of the space.
///
/// A counter, not a hash of the filename: the download store is keyed by token
/// and removes every entry matching one, so any two downloads sharing a token
/// destroy each other — and the same filename from two peers is an ordinary
/// thing to queue.
static NEXT_DOWNLOAD_TOKEN: AtomicU32 = AtomicU32::new(1);

fn next_download_token() -> u32 {
    NEXT_DOWNLOAD_TOKEN.fetch_add(1, Ordering::Relaxed) % 0x8000_0000
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
    /// When streaming began, so a snapshot can report a transfer rate.
    started: Instant,
}

/// Transfer rate of an in-progress upload, in bytes per second. Finished,
/// cancelled and failed uploads report zero, matching how a download reports
/// its speed only while running.
///
/// ponytail: an average over the whole transfer rather than the rolling window
/// the download path samples. Enough to fill the Speed column; sample a window
/// if the figure ever needs to track sudden stalls.
fn upload_speed(
    status: &crate::types::UploadStatus,
    bytes_sent: u64,
    started: Instant,
) -> f64 {
    if !matches!(status, crate::types::UploadStatus::InProgress) {
        return 0.0;
    }
    let elapsed = started.elapsed().as_secs_f64();
    if elapsed <= 0.0 {
        return 0.0;
    }
    bytes_sent as f64 / elapsed
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
    /// The version reported to the server on login. Defaults to the
    /// soulseek-rs major version with minor version 1; clients and bots built
    /// on this library should pick their own minor version.
    pub version: ClientVersion,
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
            version: ClientVersion::default(),
        }
    }
}

/// The client loop's mailbox. Non-exhaustive for the same reason as
/// [`ServerMessage`]: new protocol coverage adds variants.
#[derive(Debug)]
#[non_exhaustive]
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
    /// The server answered `GetUserStatus` for a user we asked about.
    UserStatusReceived {
        username: String,
        status: u32,
        privileged: bool,
    },
    /// The server answered `GetUserStats` for a user we asked about.
    UserStatsReceived {
        username: String,
        average_speed: u32,
        shared_files: u32,
        shared_folders: u32,
    },
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
    /// The server announced how many seconds must pass between wishlist
    /// searches.
    WishlistInterval(u32),
    /// Everyone the server counts as privileged; they sort ahead of others in
    /// our upload queue.
    PrivilegedUsers(Vec<String>),
    /// Seconds of our own privileges left.
    OwnPrivileges(u32),
    /// A peer asked where their queued file sits.
    PlaceInQueueRequested {
        requester_key: String,
        filename: String,
    },
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
    /// Who is in each room we have joined, kept current from the membership
    /// the server sends on join plus the later join/leave events.
    room_members: HashMap<String, Vec<String>>,
    /// What the server has told us about other users, merged across the
    /// separate status and statistics replies.
    user_info: HashMap<String, UserInfo>,
    /// Seconds the server wants between wishlist searches (code 104), once it
    /// has told us.
    wishlist_interval: Option<u32>,
    /// Everyone the server listed as privileged (code 69). They sort ahead of
    /// other peers in [`Self::upload_queue`].
    privileged_users: HashSet<String>,
    /// Seconds of our own privileges left (code 92), once we have asked.
    own_privileges: Option<u32>,
    /// Peers waiting for one of our upload slots.
    upload_queue: Vec<QueuedUpload>,
    /// Arrival counter for the queue's first-come tie-break.
    upload_seq: u64,
    /// How many uploads may be in flight at once.
    upload_slots: usize,
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

impl ClientContext {
    #[must_use]
    pub fn new() -> Self {
        let actor_system = Arc::new(ActorSystem::new());

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
            room_members: HashMap::new(),
            user_info: HashMap::new(),
            wishlist_interval: None,
            privileged_users: HashSet::new(),
            own_privileges: None,
            upload_queue: Vec::new(),
            upload_seq: 0,
            upload_slots: DEFAULT_UPLOAD_SLOTS,
            downloads: DownloadStore::new(),
            actor_system,
        }
    }

    /// Apply a chat-room event: keep the room-list snapshot and the per-room
    /// rosters current, then queue the event for the client/UI to drain.
    pub fn apply_room_event(&mut self, event: RoomEvent) {
        match &event {
            RoomEvent::List(rooms) => self.room_list.clone_from(rooms),
            RoomEvent::Joined { room, users } => {
                let mut members = users.clone();
                members.sort();
                members.dedup();
                self.room_members.insert(room.clone(), members);
            }
            RoomEvent::Left { room } => {
                self.room_members.remove(room);
            }
            RoomEvent::UserJoined { room, username } => {
                let members =
                    self.room_members.entry(room.clone()).or_default();
                if let Err(at) = members.binary_search(username) {
                    members.insert(at, username.clone());
                }
            }
            RoomEvent::UserLeft { room, username } => {
                if let Some(members) = self.room_members.get_mut(room)
                    && let Ok(at) = members.binary_search(username)
                {
                    members.remove(at);
                }
            }
            RoomEvent::Message { .. } => {}
        }
        self.room_events.push(event);
    }

    /// Record a `GetUserStatus` reply, merging it with any statistics already
    /// received for that user.
    pub fn apply_user_status(
        &mut self,
        username: String,
        status: u32,
        privileged: bool,
    ) {
        self.user_info
            .entry(username.clone())
            .or_insert_with(|| UserInfo::pending(username))
            .presence = Some(UserPresence {
            status: UserStatus::from_code(status),
            privileged,
        });
    }

    /// Forget what we know about `username`, so the next poll reports the
    /// answer to the request being made now rather than the previous one.
    pub fn invalidate_user_info(&mut self, username: &str) {
        self.user_info.remove(username);
    }

    /// Record a `GetUserStats` reply, merging it with any status already
    /// received for that user.
    pub fn apply_user_stats(
        &mut self,
        username: String,
        average_speed: u32,
        shared_files: u32,
        shared_folders: u32,
    ) {
        self.user_info
            .entry(username.clone())
            .or_insert_with(|| UserInfo::pending(username))
            .stats = Some(UserStats {
            average_speed,
            shared_files,
            shared_folders,
        });
    }

    /// What the server has said about `username` so far.
    #[must_use]
    pub fn user_info(&self, username: &str) -> Option<UserInfo> {
        self.user_info.get(username).cloned()
    }

    /// Who is currently in `room`, sorted, or empty when we are not in it.
    #[must_use]
    pub fn room_members(&self, room: &str) -> Vec<String> {
        self.room_members.get(room).cloned().unwrap_or_default()
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
    /// The port the listener actually holds, known once [`Client::connect`]
    /// has bound it.
    bound_port: Option<u16>,
    address: PeerAddress,
    username: String,
    password: String,
    version: ClientVersion,
    shared_directories: Vec<String>,
    server_handle: Option<ActorHandle<ServerMessage>>,
    context: Arc<RwLock<ClientContext>>,
    session: SessionWatch,
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
            bound_port: None,
            address: settings.server_address,
            username: settings.username,
            password: settings.password,
            version: settings.version,
            shared_directories: settings.shared_directories,
            context: Arc::new(RwLock::new(ClientContext::new())),
            server_handle: None,
            session: SessionWatch::default(),
        }
    }

    /// The username we log in as, for attributing our own messages.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The port peers can reach this client on, or `None` when it is not
    /// listening (or has not connected yet). This is the port that was really
    /// bound, which is not always the one that was configured.
    #[must_use]
    pub const fn listen_port(&self) -> Option<u16> {
        self.bound_port
    }

    /// Why the server session ended, or `None` while it is alive.
    ///
    /// A lost session sees nothing on the network, so an empty result set from
    /// one says nothing about what the network holds.
    #[must_use]
    pub fn session_loss(&self) -> Option<SessionLoss> {
        self.session.loss()
    }

    /// The directories whose files are currently shared with other peers.
    #[must_use]
    pub fn shared_directories(&self) -> Vec<String> {
        self.context
            .read_safe()
            .map(|ctx| ctx.shared_directories.clone())
            .unwrap_or_default()
    }

    /// `(folders, files)` currently shared with peers.
    #[must_use]
    pub fn shared_counts(&self) -> (u32, u32) {
        self.context.read_safe().map_or((0, 0), |ctx| {
            (ctx.shares.folder_count(), ctx.shares.file_count())
        })
    }

    /// Every upload this session knows about: the ones streaming or finished,
    /// followed by the peers still waiting for a slot, in the order they will
    /// be served.
    #[must_use]
    pub fn uploads(&self) -> Vec<crate::types::UploadInfo> {
        self.context.read_safe().map_or_else(
            |_| Vec::new(),
            |ctx| {
                let mut tokens: Vec<&u32> = ctx.active_uploads.keys().collect();
                tokens.sort_unstable();
                let mut all: Vec<crate::types::UploadInfo> = tokens
                    .into_iter()
                    .map(|token| {
                        let upload = &ctx.active_uploads[token];
                        let bytes_sent = upload
                            .bytes_sent
                            .load(std::sync::atomic::Ordering::Relaxed);
                        crate::types::UploadInfo {
                            username: upload.username.clone(),
                            filename: upload.filename.clone(),
                            size: upload.size,
                            bytes_sent,
                            speed_bytes_per_sec: upload_speed(
                                &upload.status,
                                bytes_sent,
                                upload.started,
                            ),
                            status: upload.status.clone(),
                        }
                    })
                    .collect();
                all.extend(ctx.queued_uploads());
                all
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
mod upload_queue;
mod uploads;

#[cfg(test)]
mod tests;
