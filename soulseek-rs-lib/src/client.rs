use crate::actor::ActorHandle;
use crate::actor::server_actor::{
    PeerAddress, ServerActor, ServerMessage, UserMessage,
};
use crate::download_store::{DownloadStore, collect_failed_tokens};
use crate::types::{DownloadMetadata, DownloadStatus};
use crate::utils::logger;
use crate::{
    Transfer,
    actor::{ActorSystem, peer_registry::PeerRegistry},
    error::{Result, SoulseekRs},
    peer::{ConnectionType, DownloadPeer, NewPeer, Peer, listen::Listen},
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

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub username: String,
    pub password: String,
    pub server_address: PeerAddress,
    pub enable_listen: bool,
    pub listen_port: u16,
    /// Directory whose files are shared with (uploaded to) other peers.
    /// `None` means nothing is shared.
    pub shared_directory: Option<String>,
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
            shared_directory: None,
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
    /// A direct outbound connection to this peer failed before it was
    /// established — the peer is likely firewalled, so fall back to asking the
    /// server to broker the connection. Carries the reporting actor's id.
    PeerConnectFailed(u64, String),
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
        Ok(DownloadStatus::Failed)
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

    assert!(matches!(receiver.try_recv(), Ok(DownloadStatus::Failed)));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(7)
            .unwrap()
            .status,
        DownloadStatus::Failed
    ));
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
            downloads: DownloadStore::new(),
            actor_system,
        }
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
    shared_directory: Option<String>,
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
            shared_directory: settings.shared_directory,
            context: Arc::new(RwLock::new(ClientContext::new())),
            server_handle: None,
        }
    }

    /// The directory whose files are shared with other peers, if configured.
    #[must_use]
    pub fn shared_directory(&self) -> Option<&str> {
        self.shared_directory.as_deref()
    }

    pub fn connect(&mut self) -> Result<()> {
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        let mut ctx = self.context.write_safe()?;
        ctx.sender = Some(sender.clone());
        let peer_registry = PeerRegistry::new(
            ctx.actor_system.clone(),
            sender.clone(),
            self.username.clone(),
        );
        ctx.peer_registry = Some(peer_registry);

        let listen_sender = sender.clone();

        // Scan the shared directory once into the read-only index, and report
        // the real folder/file counts to the server on login.
        let shares = match self.shared_directory.as_deref() {
            Some(dir) if !dir.trim().is_empty() => {
                match Shares::scan(std::path::Path::new(dir)) {
                    Ok(scanned) => {
                        info!(
                            "Sharing {} files in {} folders from {}",
                            scanned.file_count(),
                            scanned.folder_count(),
                            dir
                        );
                        Arc::new(scanned)
                    }
                    Err(e) => {
                        warn!("Failed to scan shared directory {}: {}", dir, e);
                        Arc::new(Shares::empty())
                    }
                }
            }
            _ => Arc::new(Shares::empty()),
        };
        let shared_folder_count = shares.folder_count();
        let shared_file_count = shares.file_count();
        ctx.shares = shares;

        let server_actor = ServerActor::new(
            self.address.clone(),
            sender,
            self.listen_port,
            self.enable_listen,
            shared_folder_count,
            shared_file_count,
        );

        self.server_handle = Some(ctx.actor_system.spawn_with_handle(
            server_actor,
            |actor, handle| {
                actor.set_self_handle(handle);
            },
        ));

        if self.enable_listen {
            let listen_port = self.listen_port;
            let client_sender = listen_sender;
            let context = self.context.clone();
            let own_username = self.username.clone();

            thread::spawn(move || {
                Listen::start(
                    listen_port,
                    client_sender,
                    context,
                    own_username,
                );
            });
        }

        Self::listen_to_client_operations(
            message_reader,
            self.context.clone(),
            self.username.clone(),
        );

        Ok(())
    }

    pub fn login(&self) -> Result<bool> {
        info!("Logging in as {}", self.username);
        if let Some(handle) = &self.server_handle {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = handle.send(ServerMessage::Login {
                username: self.username.clone(),
                password: self.password.clone(),
                response: tx,
            });

            match rx.recv() {
                Ok(result) => result,
                Err(_) => Err(SoulseekRs::Timeout),
            }
        } else {
            Err(SoulseekRs::NotConnected)
        }
    }

    /// Send a private message to another user via the server.
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn send_private_message(
        &self,
        username: &str,
        message: &str,
    ) -> Result<()> {
        let handle = self
            .server_handle
            .as_ref()
            .ok_or(SoulseekRs::NotConnected)?;
        let msg = crate::message::server::MessageFactory::build_message_user(
            username, message,
        );
        handle
            .send(ServerMessage::SendMessage(msg))
            .map_err(|_| SoulseekRs::NotConnected)?;
        Ok(())
    }

    /// Ask the server for a peer's address and open a direct control
    /// connection to it. Downloads queued for that peer are sent automatically
    /// once the connection is established.
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn connect_peer(&self, username: &str) -> Result<()> {
        let handle = self
            .server_handle
            .as_ref()
            .ok_or(SoulseekRs::NotConnected)?;
        handle
            .send(ServerMessage::GetPeerAddress(username.to_string()))
            .map_err(|_| SoulseekRs::NotConnected)?;
        Ok(())
    }

    /// Remove and return all private messages received since the last call.
    #[must_use]
    pub fn take_private_messages(&self) -> Vec<UserMessage> {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.take_private_messages(),
            Err(e) => {
                error!("[client] take_private_messages: {}", e);
                Vec::new()
            }
        }
    }

    #[allow(dead_code)]
    pub fn remove_peer(&self, username: &str) {
        let context = match self.context.read_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] remove_peer: {}", e);
                return;
            }
        };
        if let Some(ref registry) = context.peer_registry
            && let Some(handle) = registry.remove_peer(username)
        {
            let _ = handle.stop();
        }
    }

    pub fn search(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_cancel(query, timeout, None)
    }

    pub fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>> {
        info!("Searching for {}", query);

        if let Some(handle) = &self.server_handle {
            let hash = md5::md5(query);
            let token = u32::from_str_radix(&hash[0..5], 16)?;

            // Initialize new search with query string as key
            self.context.write_safe()?.searches.insert(
                query.to_string(),
                Search {
                    token,
                    results: Vec::new(),
                },
            );

            let _ = handle.send(ServerMessage::FileSearch {
                token,
                query: query.to_string(),
            });
        } else {
            return Err(SoulseekRs::NotConnected);
        }

        let start = Instant::now();
        loop {
            sleep(Duration::from_millis(100));

            // Check if cancelled
            if let Some(ref flag) = cancel_flag
                && flag.load(Ordering::Relaxed)
            {
                info!("Search cancelled by user");
                break;
            }

            // Check if timeout reached
            if start.elapsed() >= timeout {
                break;
            }
        }

        Ok(self.get_search_results(query))
    }

    #[must_use]
    pub fn get_search_results_count(&self, search_key: &str) -> usize {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| {
                ctx.searches.get(search_key).map(|s| s.results.len())
            })
            .unwrap_or(0)
    }

    #[must_use]
    pub fn get_search_results(&self, search_key: &str) -> Vec<SearchResult> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| {
                ctx.searches.get(search_key).map(|s| s.results.clone())
            })
            .unwrap_or_default()
    }

    /// Non-blocking variant that returns None if the lock is unavailable
    #[must_use]
    pub fn try_get_search_results(
        &self,
        search_key: &str,
    ) -> Option<Vec<SearchResult>> {
        self.context.try_read().ok().and_then(|ctx| {
            ctx.searches.get(search_key).map(|s| s.results.clone())
        })
    }

    #[must_use]
    pub fn get_all_searches(&self) -> HashMap<String, Search> {
        self.context
            .read_safe()
            .map(|ctx| ctx.searches.clone())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn get_all_downloads(&self) -> Vec<Download> {
        self.context
            .read_safe()
            .map(|ctx| ctx.get_downloads().clone())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn pause_download(&self, username: &str, filename: &str) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.downloads.pause_by_file(username, filename),
            Err(e) => {
                error!("[client] pause_download: {}", e);
                false
            }
        }
    }

    #[must_use]
    pub fn resume_download(&self, username: &str, filename: &str) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.downloads.resume_by_file(username, filename),
            Err(e) => {
                error!("[client] resume_download: {}", e);
                false
            }
        }
    }

    #[must_use]
    pub fn remove_queued_download(
        &self,
        username: &str,
        filename: &str,
    ) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => {
                ctx.downloads.remove_queued_by_file(username, filename)
            }
            Err(e) => {
                error!("[client] remove_queued_download: {}", e);
                false
            }
        }
    }

    pub fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        self.download_with_metadata(
            filename,
            username,
            size,
            download_directory,
            DownloadMetadata::default(),
        )
    }

    pub fn download_with_metadata(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
        metadata: DownloadMetadata,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        info!("[client] Downloading {} from {}", filename, username);

        let hash = md5::md5(&filename);
        let token = u32::from_str_radix(&hash[0..5], 16)?;

        let (download_sender, download_receiver): (
            Sender<DownloadStatus>,
            Receiver<DownloadStatus>,
        ) = mpsc::channel();

        let download = Download {
            username: username.clone(),
            filename,
            token,
            size,
            download_directory,
            status: DownloadStatus::Queued,
            sender: download_sender,
            queue_position: None,
            metadata,
        };

        let mut context = self.context.write_safe()?;
        context.add_download(download.clone());

        // If we already have a control connection to this peer, queue the
        // upload immediately. Otherwise open one directly (server GetPeerAddress
        // → outbound PeerInit → PeerConnected → the queued upload is flushed).
        let peer_registered = context
            .peer_registry
            .as_ref()
            .is_some_and(|r| r.contains(&username));
        let queued_now = peer_registered
            && context.peer_registry.as_ref().is_some_and(|r| {
                r.queue_upload(&username, download.filename.clone()).is_ok()
            });

        drop(context);

        let failed = if peer_registered {
            !queued_now
        } else {
            // No existing connection: initiate one. Only a genuinely
            // unconnected client (no server handle) fails outright here.
            self.connect_peer(&username).is_err()
        };

        if failed {
            let _ = download.sender.send(DownloadStatus::Failed);
            self.context
                .write_safe()?
                .update_download_with_status(token, DownloadStatus::Failed);
        }

        Ok((download, download_receiver))
    }

    /// Fail every still-`Queued` download for `username`, both on the caller's
    /// status channel (so a blocked `Receiver` unblocks) and in the store.
    fn fail_queued_downloads(
        client_context: &Arc<RwLock<ClientContext>>,
        username: &str,
    ) {
        let mut context = match client_context.write_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] fail_queued_downloads write: {}", e);
                return;
            }
        };
        let doomed: Vec<(u32, Sender<DownloadStatus>)> = context
            .get_downloads()
            .iter()
            .filter(|d| {
                d.username == username
                    && matches!(d.status, DownloadStatus::Queued)
            })
            .map(|d| (d.token, d.sender.clone()))
            .collect();
        for (token, sender) in doomed {
            let _ = sender.send(DownloadStatus::Failed);
            context.update_download_with_status(token, DownloadStatus::Failed);
        }
    }

    fn process_failed_uploads(
        client_context: Arc<RwLock<ClientContext>>,
        username: &str,
        filename: Option<&str>,
    ) {
        let failed_tokens = match client_context.read_safe() {
            Ok(context) => {
                collect_failed_tokens(&context.downloads, username, filename)
            }
            Err(e) => {
                error!("[client] process_failed_uploads read: {}", e);
                return;
            }
        };

        if failed_tokens.is_empty() {
            return;
        }

        match client_context.write_safe() {
            Ok(mut context) => {
                for token in failed_tokens {
                    context
                        .downloads
                        .update_status(token, DownloadStatus::Failed);
                    context.downloads.remove(token);
                }
            }
            Err(e) => {
                error!("[client] process_failed_uploads write: {}", e);
            }
        }
    }

    fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || {
            loop {
                match reader.recv() {
                    Ok(operation) => {
                        match operation {
                            ClientOperation::ConnectToPeer(peer) => {
                                let client_context_clone =
                                    client_context.clone();
                                let own_username_clone = own_username.clone();

                                thread::spawn(move || {
                                    Self::connect_to_peer(
                                        peer,
                                        client_context_clone,
                                        own_username_clone,
                                        None,
                                    );
                                });
                            }
                            ClientOperation::SearchResult(search_result) => {
                                trace!(
                                    "[client] SearchResult {:?}",
                                    search_result
                                );
                                let mut context = match client_context
                                    .write_safe()
                                {
                                    Ok(c) => c,
                                    Err(e) => {
                                        error!(
                                            "[client] SearchResult write: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };
                                let result_token = search_result.token;

                                // Find the search with matching token
                                for search in context.searches.values_mut() {
                                    if search.token == result_token {
                                        search.results.push(search_result);
                                        break;
                                    }
                                }
                            }
                            ClientOperation::PeerDisconnected(
                                id,
                                username,
                                error,
                            ) => {
                                // Scope the read guard: process_failed_uploads
                                // below acquires a write lock on the same
                                // RwLock, which would self-deadlock the entire
                                // client ops loop if this read guard were still
                                // held on this thread. Evict only if this exact
                                // actor still occupies the slot, so a replaced
                                // actor's shutdown can't remove its successor.
                                {
                                    let context = match client_context
                                        .read_safe()
                                    {
                                        Ok(c) => c,
                                        Err(e) => {
                                            error!(
                                                "[client] PeerDisconnected read: {}",
                                                e
                                            );
                                            continue;
                                        }
                                    };
                                    if let Some(ref registry) =
                                        context.peer_registry
                                        && let Some(handle) = registry
                                            .remove_peer_if(&username, id)
                                    {
                                        let _ = handle.stop();
                                    }
                                }
                                if let Some(error) = error {
                                    warn!(
                                        "[client] Peer {} disconnected with error: {:?}",
                                        username, error
                                    );
                                    Self::process_failed_uploads(
                                        client_context.clone(),
                                        &username,
                                        None,
                                    );
                                }
                            }
                            ClientOperation::PierceFireWall(peer) => {
                                Self::pierce_firewall(
                                    peer,
                                    client_context.clone(),
                                    own_username.clone(),
                                );
                            }
                            ClientOperation::DownloadFromPeer(
                                token,
                                peer,
                                allowed,
                            ) => {
                                let maybe_download = match client_context
                                    .read_safe()
                                {
                                    Ok(ctx) => ctx
                                        .get_download_by_token(token)
                                        .cloned(),
                                    Err(e) => {
                                        error!(
                                            "[client] DownloadFromPeer read: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };
                                let own_username = own_username.clone();
                                let client_context_clone =
                                    client_context.clone();

                                trace!(
                                    "[client] DownloadFromPeer token: {} peer: {:?}",
                                    token, peer
                                );
                                match maybe_download {
                                    Some(download) => {
                                        thread::spawn(move || {
                                            let download_peer =
                                                DownloadPeer::new(
                                                    download.username.clone(),
                                                    peer.host.clone(),
                                                    peer.port,
                                                    token,
                                                    allowed,
                                                    own_username,
                                                );
                                            let filename: Option<&str> =
                                                download
                                                    .filename
                                                    .split('\\')
                                                    .next_back();
                                            match filename {
                                                Some(filename) => {
                                                    match download_peer
                                                        .download_file(
                                                        client_context_clone
                                                            .clone(),
                                                        Some(download.clone()),
                                                        None,
                                                    ) {
                                                        Ok((
                                                            download,
                                                            filename,
                                                        )) => {
                                                            let _ = download.sender.send(DownloadStatus::Completed);
                                                            match client_context_clone.write_safe() {
                                                                Ok(mut ctx) => ctx.update_download_with_status(download.token, DownloadStatus::Completed),
                                                                Err(e) => error!("[client] download complete write: {}", e),
                                                            }
                                                            info!(
                                                                "Successfully downloaded {} bytes to {}",
                                                                download.size,
                                                                filename
                                                            );
                                                        }
                                                        Err(e) => {
                                                            let _ = download.sender.send(DownloadStatus::Failed);
                                                            match client_context_clone.write_safe() {
                                                                Ok(mut ctx) => ctx.update_download_with_status(download.token, DownloadStatus::Failed),
                                                                Err(e) => error!("[client] download failed write: {}", e),
                                                            }
                                                            error!(
                                                                "Failed to download file '{}' from {}:{} (token: {}) - Error: {}",
                                                                filename,
                                                                peer.host,
                                                                peer.port,
                                                                download.token,
                                                                e
                                                            );
                                                        }
                                                    }
                                                }
                                                None => error!(
                                                    "Cant find filename to save download: {:?}",
                                                    download.filename
                                                ),
                                            }
                                        });
                                    }
                                    None => {
                                        error!(
                                            "Can't find download with token {:?}",
                                            token
                                        );
                                    }
                                }
                            }
                            ClientOperation::NewPeer(new_peer) => {
                                let peer_exists = match client_context
                                    .read_safe()
                                {
                                    Ok(ctx) => {
                                        ctx.peer_registry.as_ref().is_some_and(
                                            |r| r.contains(&new_peer.username),
                                        )
                                    }
                                    Err(e) => {
                                        error!("[client] NewPeer read: {}", e);
                                        continue;
                                    }
                                };

                                if peer_exists {
                                    debug!(
                                        "Already connected to {}",
                                        new_peer.username
                                    );
                                } else {
                                    let send_result = client_context
                                        .read_safe()
                                        .ok()
                                        .and_then(|ctx| {
                                            ctx.server_sender.as_ref().map(
                                                |s| {
                                                    s.send(
                                                        ServerMessage::GetPeerAddress(
                                                            new_peer
                                                                .username
                                                                .clone(),
                                                        ),
                                                    )
                                                },
                                            )
                                        });
                                    if let Some(Err(e)) = send_result {
                                        error!(
                                            "[client] NewPeer send GetPeerAddress: {}",
                                            e
                                        );
                                    }
                                }

                                let addr = match new_peer.tcp_stream.peer_addr()
                                {
                                    Ok(a) => a,
                                    Err(e) => {
                                        error!(
                                            "[client] NewPeer peer_addr: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };
                                let host = addr.ip().to_string();
                                let port: u32 = addr.port().into();

                                let peer = Peer {
                                    username: new_peer.username.clone(),
                                    connection_type: new_peer.connection_type,
                                    host,
                                    port,
                                    token: Some(new_peer.token),
                                    privileged: None,
                                    obfuscated_port: None,
                                    unknown: None,
                                };

                                Self::connect_to_peer(
                                    peer,
                                    client_context.clone(),
                                    own_username.clone(),
                                    Some(new_peer.tcp_stream),
                                );
                            }
                            ClientOperation::GetPeerAddressResponse {
                                username,
                                host,
                                port,
                                obfuscation_type,
                                obfuscated_port,
                            } => {
                                debug!(
                                    "Received peer address for {}: {}:{} (obf_type: {}, obf_port: {})",
                                    username,
                                    host,
                                    port,
                                    obfuscation_type,
                                    obfuscated_port
                                );

                                let peer_exists = match client_context
                                    .read_safe()
                                {
                                    Ok(ctx) => ctx
                                        .peer_registry
                                        .as_ref()
                                        .is_some_and(|r| r.contains(&username)),
                                    Err(e) => {
                                        error!(
                                            "[client] GetPeerAddressResponse read: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };

                                if peer_exists {
                                    // Existing peer: skip re-registration. Reconnect
                                    // policy on conflict is intentionally undecided.
                                } else {
                                    let peer = Peer::new(
                                        username,
                                        ConnectionType::P,
                                        host,
                                        port,
                                        None,
                                        0,
                                        // obfuscation_type is a small enum; a
                                        // real obfuscated_port is a full u16 and
                                        // must not be truncated into a u8 (which
                                        // panicked and took down the ops thread).
                                        u8::try_from(obfuscation_type)
                                            .unwrap_or(0),
                                        obfuscated_port,
                                    );
                                    let client_context_clone =
                                        client_context.clone();
                                    let own_username_clone =
                                        own_username.clone();

                                    thread::spawn(move || {
                                        Self::connect_to_peer(
                                            peer,
                                            client_context_clone,
                                            own_username_clone,
                                            None,
                                        );
                                    });
                                }
                            }
                            ClientOperation::UpdateDownloadTokens(
                                transfer,
                                username,
                            ) => {
                                let mut context = match client_context
                                    .write_safe()
                                {
                                    Ok(c) => c,
                                    Err(e) => {
                                        error!(
                                            "[client] UpdateDownloadTokens write: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };

                                let download_to_update = context
                                    .get_downloads()
                                    .iter()
                                    .find_map(|d| {
                                        if d.username == username
                                            && d.filename == transfer.filename
                                        {
                                            Some((d.token, d.clone()))
                                        } else {
                                            None
                                        }
                                    });

                                if let Some((old_token, download)) =
                                    download_to_update
                                {
                                    trace!(
                                        "[client] UpdateDownloadTokens found {old_token}, transfer: {:?}",
                                        transfer
                                    );

                                    context.add_download(Download {
                                        username: username.clone(),
                                        filename: transfer.filename,
                                        token: transfer.token,
                                        size: transfer.size,
                                        download_directory: download
                                            .download_directory,
                                        status: download.status.clone(),
                                        sender: download.sender.clone(),
                                        queue_position: download.queue_position,
                                        metadata: download.metadata.clone(),
                                    });
                                    context.remove_download(old_token);
                                }
                            }
                            ClientOperation::UploadFailed(
                                username,
                                filename,
                            ) => {
                                Self::process_failed_uploads(
                                    client_context.clone(),
                                    &username,
                                    Some(&filename),
                                );
                            }
                            ClientOperation::PlaceInQueueUpdate {
                                username,
                                filename,
                                place,
                            } => match client_context.write_safe() {
                                Ok(mut ctx) => {
                                    let updated =
                                        ctx.downloads.update_queue_position(
                                            &username, &filename, place,
                                        );
                                    if !updated {
                                        debug!(
                                            "[client] PlaceInQueueUpdate: no matching download for {}/{}",
                                            username, filename
                                        );
                                    }
                                }
                                Err(e) => error!(
                                    "[client] PlaceInQueueUpdate write: {}",
                                    e
                                ),
                            },
                            ClientOperation::SetServerSender(sender) => {
                                match client_context.write_safe() {
                                    Ok(mut ctx) => {
                                        ctx.server_sender = Some(sender);
                                        debug!(
                                            "[client] Server sender initialized"
                                        );
                                    }
                                    Err(e) => error!(
                                        "[client] SetServerSender write: {}",
                                        e
                                    ),
                                }
                            }
                            ClientOperation::PrivateMessageReceived(
                                user_message,
                            ) => match client_context.write_safe() {
                                Ok(mut ctx) => {
                                    ctx.push_private_message(user_message);
                                }
                                Err(e) => error!(
                                    "[client] PrivateMessageReceived write: {}",
                                    e
                                ),
                            },
                            ClientOperation::PeerConnected(username) => {
                                // An outbound control connection just handshook.
                                // Flush any downloads that were queued for this
                                // peer while we were still connecting. Collect
                                // under a read guard, then act without it held.
                                let (registry, files): (
                                    Option<PeerRegistry>,
                                    Vec<String>,
                                ) = match client_context.read_safe() {
                                    Ok(ctx) => (
                                        ctx.peer_registry.clone(),
                                        ctx.get_downloads()
                                            .iter()
                                            .filter(|d| {
                                                d.username == username
                                                    && matches!(
                                                        d.status,
                                                        DownloadStatus::Queued
                                                    )
                                            })
                                            .map(|d| d.filename.clone())
                                            .collect(),
                                    ),
                                    Err(e) => {
                                        error!(
                                            "[client] PeerConnected read: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };
                                if let Some(registry) = registry {
                                    for filename in files {
                                        let _ = registry
                                            .queue_upload(&username, filename);
                                    }
                                }
                            }
                            ClientOperation::PeerConnectFailed(
                                id,
                                username,
                            ) => {
                                // Direct connect failed: ask the server to
                                // broker it. Register a correlation token, then
                                // send ConnectToPeer so the (firewalled) peer
                                // connects back to our listener quoting it.
                                let token = next_connect_token();
                                let server_sender = match client_context
                                    .write_safe()
                                {
                                    Ok(mut ctx) => {
                                        // Reap the dead outbound actor so it
                                        // stops pinning a pool worker and no
                                        // longer shadows the brokered reconnect
                                        // (a stale registry entry would make
                                        // later downloads queue into a dead,
                                        // streamless actor and hang). Identity-
                                        // aware so a newer namesake is untouched.
                                        if let Some(handle) = ctx
                                            .peer_registry
                                            .as_ref()
                                            .and_then(|r| {
                                                r.remove_peer_if(&username, id)
                                            })
                                        {
                                            let _ = handle.stop();
                                        }
                                        ctx.add_pending_connect(
                                            token,
                                            username.clone(),
                                        );
                                        ctx.server_sender.clone()
                                    }
                                    Err(e) => {
                                        error!(
                                            "[client] PeerConnectFailed write: {}",
                                            e
                                        );
                                        continue;
                                    }
                                };
                                let Some(sender) = server_sender else {
                                    continue;
                                };
                                let msg = crate::message::server::MessageFactory::build_connect_to_peer(
                                    token,
                                    &username,
                                    ConnectionType::P,
                                );
                                let _ = sender
                                    .send(ServerMessage::SendMessage(msg));

                                // Bound the brokered attempt: if no PierceFirewall
                                // consumes the token, fail the peer's queued
                                // downloads (so the caller's Receiver unblocks)
                                // and reclaim the token. A successful pierce
                                // takes the token first, making this a no-op.
                                let timeout_ctx = client_context.clone();
                                let timeout_user = username.clone();
                                thread::spawn(move || {
                                    sleep(BROKER_CONNECT_TIMEOUT);
                                    let still_pending = timeout_ctx
                                        .write_safe()
                                        .is_ok_and(|mut c| {
                                            c.take_pending_connect(token)
                                                .is_some()
                                        });
                                    if still_pending {
                                        Self::fail_queued_downloads(
                                            &timeout_ctx,
                                            &timeout_user,
                                        );
                                    }
                                });
                            }
                        }
                    }
                    Err(e) => {
                        error!("[client] Channel recv error: {:?}", e);
                        break;
                    }
                }
            }
        });
    }

    fn connect_to_peer(
        peer: Peer,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
        stream: Option<TcpStream>,
    ) {
        let client_context = client_context;

        let peer_clone = peer.clone();
        trace!(
            "[client] connecting to {}, with connection_type: {}, and token {:?}",
            peer.username, peer.connection_type, peer.token
        );
        match peer.connection_type {
            ConnectionType::P => {
                let username = peer.username;

                let context = match client_context.read_safe() {
                    Ok(c) => c,
                    Err(e) => {
                        error!("[client] connect_to_peer read: {}", e);
                        return;
                    }
                };
                if let Some(ref registry) = context.peer_registry {
                    match registry.register_peer(peer_clone, stream, None) {
                        Ok(_) => (),
                        Err(e) => {
                            trace!(
                                "Failed to spawn peer actor for {:?}: {:?}",
                                username, e
                            );
                        }
                    }
                } else {
                    trace!("PeerRegistry not initialized");
                }
            }

            ConnectionType::F => {
                trace!(
                    "[client] downloading from: {}, {:?}",
                    peer.username, peer.token
                );
                let download_peer = DownloadPeer::new(
                    peer.username,
                    peer.host,
                    peer.port,
                    peer.token.unwrap(),
                    false,
                    own_username,
                );

                match download_peer.download_file(
                    client_context.clone(),
                    None,
                    None,
                ) {
                    Ok((download, filename)) => {
                        trace!(
                            "[client] downloaded {} bytes {:?} ",
                            filename, download.size
                        );
                        let _ = download.sender.send(DownloadStatus::Completed);
                        match client_context.write_safe() {
                            Ok(mut ctx) => ctx.update_download_with_status(
                                download.token,
                                DownloadStatus::Completed,
                            ),
                            Err(e) => error!(
                                "[client] connect_to_peer F write: {}",
                                e
                            ),
                        }
                    }
                    Err(e) => {
                        trace!("[client] failed to download: {}", e);
                    }
                }
            }
            ConnectionType::D => {
                error!("ConnectionType::D not implemented");
            }
        }
    }
    fn pierce_firewall(
        peer: Peer,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        debug!("Piercing firewall for peer: {:?}", peer);

        let context = match client_context.read_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] pierce_firewall read: {}", e);
                return;
            }
        };
        if let Some(server_sender) = &context.server_sender {
            if let Some(token) = peer.token {
                match server_sender.send(ServerMessage::PierceFirewall(token)) {
                    Ok(()) => (),
                    Err(e) => {
                        error!("Failed to send PierceFirewall message: {}", e);
                    }
                }
            } else {
                error!("No token available for PierceFirewall");
            }
        } else {
            error!("No server sender available for PierceFirewall");
        }

        drop(context);
        Self::connect_to_peer(peer, client_context, own_username, None);
    }
}
