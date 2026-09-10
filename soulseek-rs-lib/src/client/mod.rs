use crate::actor::ActorHandle;
use crate::actor::server_actor::{
    PeerAddress, ServerActor, ServerMessage, UserMessage,
};
use crate::download_store::{DownloadStore, collect_failed_tokens};
use crate::types::{
    ClientVersion, DownloadMetadata, DownloadStatus, Recommendation, RoomEvent,
    RoomInfo, RoomTicker, RoomUserStats, SessionLoss, SessionWatch,
    SimilarUser, UserInfo, UserInterests, UserPresence, UserStats, UserStatus,
};
use crate::utils::logger;
use crate::{
    Transfer,
    actor::{ActorSystem, peer_registry::PeerRegistry},
    error::{Result, SoulseekRs},
    message::peer::{FileEntry, SharedDirectory, build_file_search_response},
    peer::{ConnectionType, DownloadPeer, Peer, PeerMessage, listen::Listen},
    shares::Shares,
    types::{Download, Search, SearchResult},
    utils::lock::RwLockExt,
};
use std::{
    collections::{HashMap, HashSet},
    net::TcpStream,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, sleep},
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

const DEFAULT_MAX_PEERS: usize = 512;
/// How long a message waits for a control connection to its peer.
const PENDING_PEER_TTL: Duration = Duration::from_mins(1);

const BROWSE_PROTECT_WINDOW: Duration = Duration::from_mins(5);

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

/// Source of search tokens.
///
/// A counter, not a hash of the query: a peer's answer is routed to the first
/// search holding its token, so two queries sharing one would pour results
/// into each other.
static NEXT_SEARCH_TOKEN: AtomicU32 = AtomicU32::new(1);

fn next_search_token() -> u32 {
    NEXT_SEARCH_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// A file we have agreed to serve to a peer, awaiting their TransferResponse.
struct UploadJob {
    downloader: String,
    real_path: std::path::PathBuf,
    virtual_path: String,
    size: u64,
    /// When the offer went out; `None` once the peer has answered it.
    offered: Option<Instant>,
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
/// an average over the whole transfer rather than the rolling window
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
// One reply's worth of state: the shares to search, who we are, and the
// figures the reply advertises. Kept as arguments rather than a struct
// because every one of them is read straight from the client context at the
// call site, and a struct would only move the same list one line up.
#[allow(clippy::too_many_arguments)]
fn build_search_response(
    shares: &Shares,
    own_username: &str,
    token: u32,
    query: &str,
    free_slot: bool,
    speed: u32,
    queue_length: u32,
    excluded_phrases: &[String],
) -> Option<crate::message::Message> {
    let matches = shares.search(query);
    if matches.is_empty() {
        return None;
    }
    // A file whose path carries a phrase the server excludes (code 160) is
    // left out of the reply, the way Nicotine+ does it: the exclusion is the
    // server policing what travels the search network, and answering with one
    // anyway is what it is asking us not to do.
    let entries: Vec<FileEntry> = matches
        .iter()
        .filter(|f| {
            !path_carries_excluded_phrase(&f.virtual_path, excluded_phrases)
        })
        .take(crate::types::MAX_SEARCH_REPLY_FILES)
        .map(|f| FileEntry {
            name: &f.virtual_path,
            size: f.size,
            attribs: &f.attributes,
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(build_file_search_response(
        own_username,
        token,
        &entries,
        u8::from(free_slot),
        speed,
        queue_length,
    ))
}

/// Whether `path` contains any of the phrases the server excludes. Matching
/// is on the lowercased path, as the phrases themselves are lowercase.
fn path_carries_excluded_phrase(path: &str, excluded: &[String]) -> bool {
    if excluded.is_empty() {
        return false;
    }
    let lowered = path.to_lowercase();
    excluded
        .iter()
        .any(|phrase| !phrase.is_empty() && lowered.contains(phrase))
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
    /// Whether to serve children in the distributed search network: peers
    /// hang from us with a `D` connection and we pass every search we receive
    /// down to them. Off by default — it costs a socket and the network's
    /// whole search stream per child.
    pub accept_children: bool,
    /// The version reported to the server on login. Defaults to the
    /// soulseek-rs major version with minor version 1 ("unidentified");
    /// clients built on this library should reserve their own minor
    /// version (see [`ClientVersion`]).
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
            accept_children: false,
            version: ClientVersion::default(),
        }
    }
}

/// The client loop's mailbox. Non-exhaustive for the same reason as
/// [`ServerMessage`]: new protocol coverage adds variants.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClientOperation {
    ConnectToPeer(Peer),
    SearchResult(SearchResult),
    PeerDisconnected(u64, String, Option<SoulseekRs>),
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
    /// The server answered `WatchUser` for a user we started watching.
    WatchedUserReceived {
        username: String,
        exists: bool,
        status: Option<u32>,
        average_speed: Option<u32>,
        shared_files: Option<u32>,
        shared_folders: Option<u32>,
    },
    PeerConnected(String),
    /// A search distributed to us by the server; reply if our shares match.
    IncomingSearch {
        username: String,
        token: u32,
        query: String,
        /// True when it came down the tree from our parent, which has
        /// already passed it to our children and is subject to the answer
        /// budget. A search the server relayed is neither.
        from_parent: bool,
    },
    /// A peer queued one of our shared files; `requester_key` is the registry
    /// key of the peer actor — the peer's username.
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
    /// A peer asked what we say about ourselves; send a UserInfoResponse.
    UserInfoRequested {
        requester_key: String,
    },
    /// A peer asked for one folder of our shares; send a FolderContentsResponse.
    FolderContentsRequested {
        requester_key: String,
        token: u32,
        folder: String,
    },
    /// A peer answered our request for what it says about itself.
    PeerInfoReceived {
        username: String,
        info: crate::message::peer::PeerInfo,
    },
    /// A peer answered our request for one folder of their shares.
    FolderContents {
        username: String,
        token: u32,
        folder: String,
        directories: Vec<SharedDirectory>,
    },
    /// A peer we are browsing returned their shared-file listing.
    BrowseResult {
        username: String,
        directories: Vec<SharedDirectory>,
    },
    /// A direct outbound connection to this peer failed before it was
    /// established — the peer is likely firewalled, so fall back to asking the
    /// server to broker the connection. Carries the reporting actor's id and,
    /// when the dial was one the server asked us to make, the token that peer
    /// quoted: that dial is answered with a `CantConnectToPeer` instead of a
    /// broker request, since the peer is already waiting on the server.
    PeerConnectFailed(u64, String, Option<u32>),
    /// Something happened in the chat-room subsystem (list refreshed, a room
    /// joined/left, a message said, a member joined/left).
    RoomEvent(RoomEvent),
    /// Per-member statistics from a `JoinRoom` reply.
    RoomMemberStats {
        room: String,
        stats: Vec<RoomUserStats>,
    },
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
    /// Distributed-network parent candidates, from the server or a host.
    PossibleParents(Vec<(String, String, u16)>),
    /// The server told us to drop our parent.
    ResetDistributed,
    /// A session just started: anything the server keeps only per-session —
    /// our interests — has to be sent again.
    SessionEstablished,
    /// A peer dialled us with a `D` connection, asking to hang from us in the
    /// distributed search network.
    ChildConnected {
        username: String,
        stream: std::net::TcpStream,
    },
    /// Recommendations from the server: either from our own interests
    /// (`global` false, code 54) or server-wide (`global` true, code 56).
    Recommendations {
        global: bool,
        recommended: Vec<Recommendation>,
        unrecommended: Vec<Recommendation>,
    },
    /// Recommendations for one item (code 111).
    ItemRecommendations {
        item: String,
        recommendations: Vec<Recommendation>,
    },
    /// Users the server considers similar to us (code 110).
    SimilarUsers(Vec<SimilarUser>),
    /// Users who like one item (code 112).
    ItemSimilarUsers {
        item: String,
        usernames: Vec<String>,
    },
    /// What another user likes and hates (code 57).
    UserInterests(UserInterests),
    /// A peer we asked the server to broker gave up reaching us (code 1001).
    CantConnectToPeer {
        token: u32,
    },
    /// Phrases the server refuses to search for (code 160).
    ExcludedSearchPhrases(Vec<String>),
    /// The minimum upload speed for carrying children (code 83).
    ParentMinSpeed(u32),
    /// The divisor turning that speed into a child count (code 84).
    ParentSpeedRatio(u32),
    /// A parent candidate told us how deep it sits. `link` says which dial
    /// to that user is talking.
    ParentBranchLevel {
        parent: String,
        link: u64,
        level: i32,
    },
    /// A parent candidate told us whose branch it is on.
    ParentBranchRoot {
        parent: String,
        link: u64,
        root: String,
    },
    /// A search came down the tree from `parent`.
    ParentSearch {
        parent: String,
        link: u64,
        username: String,
        token: u32,
        query: String,
    },
    /// The link to a parent or candidate is gone.
    ParentClosed {
        parent: String,
        link: u64,
    },
}
pub struct ClientContext {
    pub peer_registry: Option<PeerRegistry>,
    pub downloads: DownloadStore,
    server_sender: Option<Sender<ServerMessage>>,
    /// Where anything outside the operations loop posts operations.
    operations: Option<Sender<ClientOperation>>,
    /// Our place in the distributed search network.
    leaf: distributed::Leaf,
    /// The children hanging from us, when the client serves any.
    pub(crate) children: children::Children,
    /// The last `AcceptChildren` answer we gave the server, so the same
    /// answer is not repeated: it is a standing state, not a heartbeat.
    announced_accept_children: Option<bool>,
    /// The upload speed the server records for us, once it has told us. The
    /// child limit is derived from it.
    own_average_speed: Option<u32>,
    /// The speed a parent needs (code 83) and the divisor turning speed into
    /// a child count (code 84), as the server announced them.
    parent_min_speed: u32,
    parent_speed_ratio: u32,
    searches: HashMap<String, Search>,
    private_messages: Vec<UserMessage>,
    /// Correlation tokens for server-brokered (firewalled) connections, mapping
    /// a token we sent in a ConnectToPeer to the peer we expect back.
    pending_connect_tokens: HashMap<u32, (String, Instant)>,
    max_peers: Arc<AtomicUsize>,
    /// Files we share with peers (read-only after connect).
    pub shares: Arc<Shares>,
    /// The directories the current share index was built from.
    pub shared_directories: Vec<String>,
    /// Peer listen addresses learned from GetPeerAddress responses.
    peer_addresses: HashMap<String, (String, u32)>,
    /// Peer messages waiting for a control connection to that peer, and
    /// when the last was queued: a peer that never answers must not keep
    /// them forever.
    pending_peer_messages:
        HashMap<String, (Instant, Vec<crate::message::Message>)>,
    /// Uploads we have offered, keyed by our transfer token.
    uploads: HashMap<u32, UploadJob>,
    active_uploads: HashMap<u32, ActiveUpload>,
    /// Upload tokens waiting for the downloader's address to be resolved.
    pending_serves: HashMap<String, Vec<u32>>,
    /// Shared-file listings received from peers we browsed.
    browse_results: HashMap<String, Vec<SharedDirectory>>,
    /// One-folder listings received from peers, keyed by peer and folder.
    folder_contents: HashMap<(String, String), Vec<SharedDirectory>>,
    /// What peers said about themselves (peer code 16), keyed by peer.
    peer_infos: HashMap<String, crate::message::peer::PeerInfo>,
    pending_browses: HashMap<String, Instant>,
    /// Latest snapshot of the public chat-room list (from `RoomList`, code 64).
    room_list: Vec<RoomInfo>,
    /// Chat-room events awaiting consumption by the client/UI.
    room_events: Vec<RoomEvent>,
    /// Who is in each room we have joined, kept current from the membership
    /// the server sends on join plus the later join/leave events.
    room_members: HashMap<String, Vec<String>>,
    /// Per-member statistics for each joined room, from the stat vectors the
    /// server sends alongside the membership list.
    room_member_stats: HashMap<String, Vec<RoomUserStats>>,
    /// Who may enter each private room we belong to (code 133), kept current
    /// from the later roster changes.
    private_room_members: HashMap<String, Vec<String>>,
    /// Who runs each of those rooms (code 148).
    private_room_operators: HashMap<String, Vec<String>>,
    /// The ticker board of each joined room, kept current from the board sent
    /// on join (code 113) and the later add/remove events (114/115).
    room_tickers: HashMap<String, Vec<RoomTicker>>,
    /// The latest recommendations from our own interests (code 54), as
    /// (recommended, recommended-against).
    recommendations: Option<(Vec<Recommendation>, Vec<Recommendation>)>,
    /// The latest server-wide recommendations (code 56).
    global_recommendations: Option<(Vec<Recommendation>, Vec<Recommendation>)>,
    /// Per-item recommendations (code 111), keyed by the item asked about.
    item_recommendations: HashMap<String, Vec<Recommendation>>,
    /// The latest similar-user answer (code 110).
    similar_users: Vec<SimilarUser>,
    /// Who likes an item (code 112), keyed by the item asked about.
    item_similar_users: HashMap<String, Vec<String>>,
    /// Interests of other users (code 57), keyed by username.
    user_interests: HashMap<String, UserInterests>,
    /// What we ourselves like and hate. The server keeps these only for the
    /// duration of a session, so they are held here and sent again after each
    /// login — the same thing Nicotine+ does from its config.
    own_interests: UserInterests,
    /// What the server has told us about other users, merged across the
    /// separate status and statistics replies.
    user_info: HashMap<String, UserInfo>,
    /// Users we asked the server to watch (code 5), so it keeps pushing their
    /// status changes. Kept so a UI can render the watch list and so an
    /// unwatch can be rejected for someone we never watched.
    watched_users: HashSet<String>,
    /// Seconds the server wants between wishlist searches (code 104), once it
    /// has told us.
    wishlist_interval: Option<u32>,
    /// Everyone the server listed as privileged (code 69). They sort ahead of
    /// other peers in [`Self::upload_queue`].
    privileged_users: HashSet<String>,
    /// Seconds of our own privileges left (code 92), once we have asked.
    own_privileges: Option<u32>,
    /// Phrases the server excludes from the search network (code 160). Files
    /// whose path carries one are left out of the replies we send.
    excluded_search_phrases: Vec<String>,
    /// Peers waiting for one of our upload slots.
    upload_queue: Vec<QueuedUpload>,
    /// Arrival counter for the queue's first-come tie-break.
    upload_seq: u64,
    /// How many uploads may be in flight at once.
    upload_slots: usize,
    /// Bytes per second of the last completed upload, advertised in search
    /// replies; zero until one has finished.
    last_upload_speed: u32,
    /// Queued-upload states that came and went between two polls of
    /// [`Client::uploads`]. A caller sampling that snapshot would otherwise
    /// never see a peer that queued and was served inside one poll interval,
    /// and "it waited" is exactly the fact a transfer log must not lose.
    upload_events: Vec<crate::types::UploadInfo>,
    actor_system: Arc<ActorSystem>,
}
/// A roster sorted and de-duplicated, so membership lookups can binary-search
/// it and a repeated name cannot appear twice.
fn sorted_unique(users: &[String]) -> Vec<String> {
    let mut users = users.to_vec();
    users.sort();
    users.dedup();
    users
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
    /// Tells the peer listener to stop, so a disconnected client releases the
    /// port it bound.
    listener_stopped: Arc<AtomicBool>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.disconnect();
    }
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
        let mut context = ClientContext::for_user(&settings.username);
        context.children = children::Children::new(settings.accept_children);
        Self {
            enable_listen: settings.enable_listen,
            listen_port: settings.listen_port,
            bound_port: None,
            address: settings.server_address,
            context: Arc::new(RwLock::new(context)),
            username: settings.username,
            password: settings.password,
            version: settings.version,
            shared_directories: settings.shared_directories,
            server_handle: None,
            session: SessionWatch::default(),
            listener_stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The children currently hanging from us in the distributed search
    /// network, by username. Empty unless the client was built with
    /// [`ClientSettings::accept_children`].
    #[must_use]
    pub fn children(&self) -> Vec<String> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.children.usernames(),
            Err(e) => {
                error!("[client] children: {}", e);
                Vec::new()
            }
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

    /// The share index as a peer receives it: the same directory listing
    /// `GetShareFileList` answers with, so what this returns is what the
    /// network sees.
    #[must_use]
    pub fn shared_listing(&self) -> Vec<crate::message::peer::SharedDirectory> {
        self.context
            .read_safe()
            .map(|ctx| ctx.shares.directories())
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

mod children;
mod connection;
mod context;
mod distributed;
mod downloads;
mod operations;
mod rooms;
mod search;
mod social;
mod upload_queue;
mod uploads;

#[cfg(test)]
mod tests;
