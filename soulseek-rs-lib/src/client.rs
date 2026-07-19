use crate::actor::server_actor::{
    PeerAddress, ServerActor, ServerMessage, UserMessage,
};
use crate::actor::{Actor, ActorHandle};
use crate::download_store::{DownloadStore, collect_failed_tokens};
use crate::message::MessageReader;
use crate::types::{DownloadMetadata, DownloadStatus};
use crate::utils::logger;
use crate::{
    Transfer,
    actor::{ActorSystem, peer_registry::PeerRegistry},
    error::{Result, SoulseekRs},
    peer::{ConnectionType, DownloadPeer, NewPeer, Peer, listen::ListenActor},
    types::{Download, Search, SearchResult},
    utils::{lock::RwLockExt, md5},
};
use std::{
    collections::HashMap,
    net::TcpStream,
    sync::{
        Mutex, RwLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, Sender, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace, warn};
const DEFAULT_LISTEN_PORT: u16 = 2234;
const BROKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

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

pub enum ClientOperation {
    NewPeer(NewPeer),
    PeerConnection {
        peer: Peer,
        stream: TcpStream,
        reader: MessageReader,
    },
    IncomingFileConnection {
        peer: Peer,
        stream: TcpStream,
        reader: MessageReader,
        token: u32,
        peer_ip: String,
        peer_port: u16,
    },
    PierceFirewallConnection {
        token: u32,
        stream: TcpStream,
        reader: MessageReader,
        peer_ip: String,
        peer_port: u16,
    },
    ConnectToPeer(Peer),
    StartSearch {
        query: String,
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
        response: Sender<Result<Vec<SearchResult>>>,
    },
    SearchResult(SearchResult),
    PeerDisconnected(u64, String, Option<SoulseekRs>),
    PierceFireWall(Peer),
    DownloadFromPeer(u32, Peer, bool),
    DownloadStatusUpdate {
        token: u32,
        status: DownloadStatus,
        notify: bool,
    },
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
    SetServerHandle(ActorHandle<ServerMessage>),
    PrivateMessageReceived(UserMessage),
    PeerConnected(String),
    PeerConnectFailed(u64, String),
}
pub struct ClientContext {
    pub peer_registry: Option<PeerRegistry>,
    pub downloads: DownloadStore,
    client_handle: Option<ActorHandle<ClientOperation>>,
    server_handle: Option<ActorHandle<ServerMessage>>,
    searches: HashMap<String, Search>,
    private_messages: Vec<UserMessage>,
    pending_connect_tokens: HashMap<u32, String>,
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
        let actor_system = Arc::new(ActorSystem::new());

        Self {
            peer_registry: None,
            client_handle: None,
            server_handle: None,
            searches: HashMap::new(),
            private_messages: Vec::new(),
            pending_connect_tokens: HashMap::new(),
            downloads: DownloadStore::new(),
            actor_system,
        }
    }

    pub fn push_private_message(&mut self, message: UserMessage) {
        self.private_messages.push(message);
    }

    #[must_use]
    pub fn take_private_messages(&mut self) -> Vec<UserMessage> {
        std::mem::take(&mut self.private_messages)
    }

    pub fn add_pending_connect(&mut self, token: u32, username: String) {
        self.pending_connect_tokens.insert(token, username);
    }

    pub fn take_pending_connect(&mut self, token: u32) -> Option<String> {
        self.pending_connect_tokens.remove(&token)
    }
}

type DownloadJob = Box<dyn FnOnce() + Send + 'static>;

const DEFAULT_DOWNLOAD_WORKERS: usize = 4;
const DEFAULT_DOWNLOAD_QUEUE_BOUND: usize = 64;

struct DownloadExecutor {
    sender: Option<SyncSender<DownloadJob>>,
    workers: Vec<JoinHandle<()>>,
}

impl DownloadExecutor {
    fn new(worker_count: usize, queue_bound: usize) -> Self {
        let worker_count = worker_count.max(1);
        let queue_bound = queue_bound.max(1);
        let (sender, receiver) = mpsc::sync_channel::<DownloadJob>(queue_bound);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(worker_count);

        for index in 0..worker_count {
            let receiver = receiver.clone();
            let worker = thread::Builder::new()
                .name(format!("soulseek-download-{index}"))
                .spawn(move || {
                    loop {
                        let job = match receiver.lock() {
                            Ok(receiver) => receiver.recv(),
                            Err(_) => return,
                        };

                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                })
                .expect("failed to spawn download worker");
            workers.push(worker);
        }

        Self {
            sender: Some(sender),
            workers,
        }
    }

    fn execute<F>(&self, job: F) -> std::result::Result<(), String>
    where
        F: FnOnce() + Send + 'static,
    {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| "download executor is stopped".to_string())?;

        match sender.try_send(Box::new(job)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                Err("download executor queue is full".to_string())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err("download executor is stopped".to_string())
            }
        }
    }
}

impl Default for DownloadExecutor {
    fn default() -> Self {
        Self::new(DEFAULT_DOWNLOAD_WORKERS, DEFAULT_DOWNLOAD_QUEUE_BOUND)
    }
}

impl Drop for DownloadExecutor {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct ClientActor {
    context: Arc<RwLock<ClientContext>>,
    own_username: String,
    download_executor: DownloadExecutor,
    self_handle: Option<ActorHandle<ClientOperation>>,
    pending_searches: Vec<PendingSearch>,
    pending_broker_connections: Vec<PendingBrokerConnection>,
}

struct PendingSearch {
    query: String,
    deadline: Instant,
    cancel_flag: Option<Arc<AtomicBool>>,
    response: Sender<Result<Vec<SearchResult>>>,
}

struct PendingBrokerConnection {
    token: u32,
    username: String,
    deadline: Instant,
}

struct IncomingFileJob {
    peer: Peer,
    stream: TcpStream,
    reader: MessageReader,
    token: u32,
    peer_ip: String,
    peer_port: u16,
    own_username: String,
    client_context: Arc<RwLock<ClientContext>>,
    client_handle: ActorHandle<ClientOperation>,
}

impl ClientActor {
    fn new(context: Arc<RwLock<ClientContext>>, own_username: String) -> Self {
        Self {
            context,
            own_username,
            download_executor: DownloadExecutor::default(),
            self_handle: None,
            pending_searches: Vec::new(),
            pending_broker_connections: Vec::new(),
        }
    }

    fn set_self_handle(&mut self, handle: ActorHandle<ClientOperation>) {
        self.self_handle = Some(handle);
    }

    fn handle_operation(&mut self, operation: ClientOperation) {
        match operation {
            ClientOperation::ConnectToPeer(peer) => {
                self.connect_or_download(peer, None, None);
            }
            ClientOperation::PeerConnection {
                peer,
                stream,
                reader,
            } => {
                self.connect_or_download(peer, Some(stream), Some(reader));
            }
            ClientOperation::IncomingFileConnection {
                peer,
                stream,
                reader,
                token,
                peer_ip,
                peer_port,
            } => {
                self.schedule_incoming_file_connection(
                    peer, stream, reader, token, peer_ip, peer_port,
                );
            }
            ClientOperation::PierceFirewallConnection {
                token,
                stream,
                reader,
                peer_ip,
                peer_port,
            } => {
                self.handle_pierce_firewall_connection(
                    token, stream, reader, peer_ip, peer_port,
                );
            }
            ClientOperation::SearchResult(search_result) => {
                trace!("[client] SearchResult {:?}", search_result);
                let mut context = match self.context.write_safe() {
                    Ok(c) => c,
                    Err(e) => {
                        error!("[client] SearchResult write: {}", e);
                        return;
                    }
                };
                let result_token = search_result.token;

                for search in context.searches.values_mut() {
                    if search.token == result_token {
                        search.results.push(search_result);
                        break;
                    }
                }
            }
            ClientOperation::PeerDisconnected(id, username, error) => {
                {
                    let context = match self.context.read_safe() {
                        Ok(c) => c,
                        Err(e) => {
                            error!("[client] PeerDisconnected read: {}", e);
                            return;
                        }
                    };
                    if let Some(ref registry) = context.peer_registry
                        && let Some(handle) =
                            registry.remove_peer_if(&username, id)
                    {
                        let _ = handle.stop();
                    }
                }
                if let Some(error) = error {
                    warn!(
                        "[client] Peer {} disconnected with error: {:?}",
                        username, error
                    );
                    Client::process_failed_uploads(
                        self.context.clone(),
                        &username,
                        None,
                    );
                }
            }
            ClientOperation::PrivateMessageReceived(user_message) => {
                match self.context.write_safe() {
                    Ok(mut ctx) => {
                        ctx.push_private_message(user_message);
                    }
                    Err(e) => {
                        error!("[client] PrivateMessageReceived write: {}", e);
                    }
                }
            }
            ClientOperation::PeerConnected(username) => {
                self.flush_queued_downloads_for_peer(&username);
            }
            ClientOperation::PeerConnectFailed(id, username) => {
                self.broker_peer_connection(id, username);
            }
            ClientOperation::PierceFireWall(peer) => {
                self.send_pierce_firewall(&peer);
                self.connect_or_download(peer, None, None);
            }
            ClientOperation::DownloadFromPeer(token, peer, allowed) => {
                self.schedule_download_from_peer(token, peer, allowed);
            }
            ClientOperation::StartSearch {
                query,
                timeout,
                cancel_flag,
                response,
            } => {
                self.start_search(query, timeout, cancel_flag, response);
            }
            ClientOperation::DownloadStatusUpdate {
                token,
                status,
                notify,
            } => {
                self.apply_download_status(token, status, notify);
            }
            ClientOperation::NewPeer(new_peer) => {
                self.handle_new_peer(new_peer);
            }
            ClientOperation::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            } => {
                self.handle_peer_address_response(
                    username,
                    host,
                    port,
                    obfuscation_type,
                    obfuscated_port,
                );
            }
            ClientOperation::UpdateDownloadTokens(transfer, username) => {
                self.update_download_tokens(transfer, username);
            }
            ClientOperation::UploadFailed(username, filename) => {
                Client::process_failed_uploads(
                    self.context.clone(),
                    &username,
                    Some(&filename),
                );
            }
            ClientOperation::PlaceInQueueUpdate {
                username,
                filename,
                place,
            } => match self.context.write_safe() {
                Ok(mut ctx) => {
                    let updated = ctx
                        .downloads
                        .update_queue_position(&username, &filename, place);
                    if !updated {
                        debug!(
                            "[client] PlaceInQueueUpdate: no matching download for {}/{}",
                            username, filename
                        );
                    }
                }
                Err(e) => {
                    error!("[client] PlaceInQueueUpdate write: {}", e);
                }
            },
            ClientOperation::SetServerHandle(handle) => {
                match self.context.write_safe() {
                    Ok(mut ctx) => {
                        ctx.server_handle = Some(handle);
                        debug!("[client] Server handle initialized");
                    }
                    Err(e) => {
                        error!("[client] SetServerHandle write: {}", e);
                    }
                }
            }
        }
    }

    fn connect_or_download(
        &self,
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
    ) {
        match peer.connection_type.clone() {
            ConnectionType::P => Client::connect_to_peer(
                peer,
                self.context.clone(),
                self.own_username.clone(),
                stream,
                reader,
                None,
            ),
            ConnectionType::F => {
                let Some(client_handle) = self.self_handle.clone() else {
                    error!("[client] missing self handle for file connection");
                    return;
                };
                let context = self.context.clone();
                let own_username = self.own_username.clone();
                if let Err(e) = self.download_executor.execute(move || {
                    Client::connect_to_peer(
                        peer,
                        context,
                        own_username,
                        stream,
                        reader,
                        Some(client_handle),
                    );
                }) {
                    error!("[client] failed to queue file connection: {}", e);
                }
            }
            ConnectionType::D => {
                error!("ConnectionType::D not implemented");
            }
        }
    }

    fn send_pierce_firewall(&self, peer: &Peer) {
        debug!("Piercing firewall for peer: {:?}", peer);

        let context = match self.context.read_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] pierce_firewall read: {}", e);
                return;
            }
        };
        if let Some(server_handle) = &context.server_handle {
            if let Some(token) = peer.token {
                if let Err(e) =
                    server_handle.send(ServerMessage::PierceFirewall(token))
                {
                    error!("Failed to send PierceFirewall message: {}", e);
                }
            } else {
                error!("No token available for PierceFirewall");
            }
        } else {
            error!("No server handle available for PierceFirewall");
        }
    }

    fn handle_pierce_firewall_connection(
        &self,
        token: u32,
        stream: TcpStream,
        reader: MessageReader,
        peer_ip: String,
        peer_port: u16,
    ) {
        let username = match self.context.write_safe() {
            Ok(mut ctx) => ctx.take_pending_connect(token),
            Err(e) => {
                error!("[client] pierce firewall token write: {}", e);
                return;
            }
        };
        let Some(username) = username else {
            debug!(
                "[listener:{peer_ip}:{peer_port}] PierceFirewall token {token} is not pending; ignoring"
            );
            return;
        };

        let peer = Peer::new(
            username.clone(),
            ConnectionType::P,
            peer_ip,
            peer_port.into(),
            None,
            0,
            0,
            0,
        );
        self.connect_or_download(peer, Some(stream), Some(reader));
        self.flush_queued_downloads_for_peer(&username);
    }

    fn flush_queued_downloads_for_peer(&self, username: &str) {
        let (registry, files): (Option<PeerRegistry>, Vec<String>) = match self
            .context
            .read_safe()
        {
            Ok(ctx) => (
                ctx.peer_registry.clone(),
                ctx.get_downloads()
                    .iter()
                    .filter(|download| {
                        download.username == username
                            && matches!(download.status, DownloadStatus::Queued)
                    })
                    .map(|download| download.filename.clone())
                    .collect(),
            ),
            Err(e) => {
                error!("[client] PeerConnected read: {}", e);
                return;
            }
        };

        if let Some(registry) = registry {
            for filename in files {
                let _ = registry.queue_upload(username, filename);
            }
        }
    }

    fn broker_peer_connection(&mut self, id: u64, username: String) {
        let token = next_connect_token();
        let server_handle = match self.context.write_safe() {
            Ok(mut ctx) => {
                if let Some(handle) = ctx
                    .peer_registry
                    .as_ref()
                    .and_then(|registry| registry.remove_peer_if(&username, id))
                {
                    let _ = handle.stop();
                }
                ctx.add_pending_connect(token, username.clone());
                ctx.server_handle.clone()
            }
            Err(e) => {
                error!("[client] PeerConnectFailed write: {}", e);
                return;
            }
        };

        let Some(server_handle) = server_handle else {
            self.cancel_pending_connect(token);
            Self::fail_queued_downloads(&self.context, &username);
            return;
        };

        let message =
            crate::message::server::MessageFactory::build_connect_to_peer(
                token,
                &username,
                ConnectionType::P,
            );
        if let Err(e) = server_handle.send(ServerMessage::SendMessage(message))
        {
            error!("[client] failed to request brokered peer connect: {}", e);
            self.cancel_pending_connect(token);
            Self::fail_queued_downloads(&self.context, &username);
            return;
        }

        self.pending_broker_connections
            .push(PendingBrokerConnection {
                token,
                username,
                deadline: Instant::now()
                    .checked_add(BROKER_CONNECT_TIMEOUT)
                    .unwrap_or_else(Instant::now),
            });
    }

    fn cancel_pending_connect(&self, token: u32) {
        match self.context.write_safe() {
            Ok(mut ctx) => {
                ctx.take_pending_connect(token);
            }
            Err(e) => {
                error!("[client] cancel pending connect write: {}", e);
            }
        }
    }

    fn complete_expired_broker_connections(&mut self) {
        let now = Instant::now();
        let mut index = 0;

        while index < self.pending_broker_connections.len() {
            if now < self.pending_broker_connections[index].deadline {
                index += 1;
                continue;
            }

            let pending = self.pending_broker_connections.swap_remove(index);
            let still_pending = match self.context.write_safe() {
                Ok(mut ctx) => {
                    ctx.take_pending_connect(pending.token).is_some()
                }
                Err(e) => {
                    error!("[client] broker timeout write: {}", e);
                    false
                }
            };

            if still_pending {
                Self::fail_queued_downloads(&self.context, &pending.username);
            }
        }
    }

    fn start_search(
        &mut self,
        query: String,
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
        response: Sender<Result<Vec<SearchResult>>>,
    ) {
        let hash = md5::md5(&query);
        let token = match u32::from_str_radix(&hash[0..5], 16) {
            Ok(token) => token,
            Err(e) => {
                let _ = response.send(Err(e.into()));
                return;
            }
        };

        let server_handle = match self.context.write_safe() {
            Ok(mut ctx) => {
                ctx.searches.insert(
                    query.clone(),
                    Search {
                        token,
                        results: Vec::new(),
                    },
                );
                ctx.server_handle.clone()
            }
            Err(e) => {
                let _ = response.send(Err(e));
                return;
            }
        };

        let Some(server_handle) = server_handle else {
            self.remove_search(&query);
            let _ = response.send(Err(SoulseekRs::NotConnected));
            return;
        };

        if let Err(e) = server_handle.send(ServerMessage::FileSearch {
            token,
            query: query.clone(),
        }) {
            self.remove_search(&query);
            let _ = response.send(Err(SoulseekRs::InvalidMessage(e)));
            return;
        }

        self.pending_searches.push(PendingSearch {
            query,
            deadline: Instant::now()
                .checked_add(timeout)
                .unwrap_or_else(Instant::now),
            cancel_flag,
            response,
        });
    }

    fn remove_search(&self, query: &str) {
        match self.context.write_safe() {
            Ok(mut ctx) => {
                ctx.searches.remove(query);
            }
            Err(e) => {
                error!("[client] remove search write: {}", e);
            }
        }
    }

    fn complete_finished_searches(&mut self) {
        let now = Instant::now();
        let mut index = 0;

        while index < self.pending_searches.len() {
            let pending = &self.pending_searches[index];
            let cancelled = pending
                .cancel_flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed));

            if !cancelled && now < pending.deadline {
                index += 1;
                continue;
            }

            if cancelled {
                info!("Search cancelled by user");
            }

            let pending = self.pending_searches.swap_remove(index);
            let result = self.search_results_for(&pending.query);
            let _ = pending.response.send(result);
        }
    }

    fn search_results_for(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.context.read_safe().map(|ctx| {
            ctx.searches
                .get(query)
                .map(|s| s.results.clone())
                .unwrap_or_default()
        })
    }

    fn schedule_download_from_peer(
        &self,
        token: u32,
        peer: Peer,
        allowed: bool,
    ) {
        let maybe_download = match self.context.read_safe() {
            Ok(ctx) => ctx.get_download_by_token(token).cloned(),
            Err(e) => {
                error!("[client] DownloadFromPeer read: {}", e);
                return;
            }
        };

        trace!(
            "[client] DownloadFromPeer token: {} peer: {:?}",
            token, peer
        );
        let Some(download) = maybe_download else {
            error!("Can't find download with token {:?}", token);
            return;
        };

        let Some(client_handle) = self.self_handle.clone() else {
            error!("[client] missing self handle for download");
            self.apply_download_status(
                download.token,
                DownloadStatus::Failed,
                true,
            );
            return;
        };
        let client_context = self.context.clone();
        let own_username = self.own_username.clone();
        let failure_token = download.token;

        if let Err(e) = self.download_executor.execute(move || {
            Self::run_download_from_peer(
                token,
                peer,
                allowed,
                download,
                own_username,
                client_context,
                client_handle,
            );
        }) {
            error!("[client] failed to queue download: {}", e);
            self.apply_download_status(
                failure_token,
                DownloadStatus::Failed,
                true,
            );
        }
    }

    fn run_download_from_peer(
        token: u32,
        peer: Peer,
        allowed: bool,
        download: Download,
        own_username: String,
        client_context: Arc<RwLock<ClientContext>>,
        client_handle: ActorHandle<ClientOperation>,
    ) {
        let download_peer = DownloadPeer::new(
            download.username.clone(),
            peer.host.clone(),
            peer.port,
            token,
            allowed,
            own_username,
        );
        let filename = download.filename.split('\\').next_back();
        match filename {
            Some(filename) => {
                match download_peer.download_file(
                    client_context,
                    client_handle.clone(),
                    Some(download.clone()),
                    None,
                ) {
                    Ok((download, filename)) => {
                        Self::send_download_status_update(
                            &client_handle,
                            download.token,
                            DownloadStatus::Completed,
                            true,
                        );
                        info!(
                            "Successfully downloaded {} bytes to {}",
                            download.size, filename
                        );
                    }
                    Err(e) => {
                        Self::send_download_status_update(
                            &client_handle,
                            download.token,
                            DownloadStatus::Failed,
                            true,
                        );
                        error!(
                            "Failed to download file '{}' from {}:{} (token: {}) - Error: {}",
                            filename, peer.host, peer.port, download.token, e
                        );
                    }
                }
            }
            None => {
                error!(
                    "Cant find filename to save download: {:?}",
                    download.filename
                );
            }
        }
    }

    fn schedule_incoming_file_connection(
        &self,
        peer: Peer,
        stream: TcpStream,
        reader: MessageReader,
        token: u32,
        peer_ip: String,
        peer_port: u16,
    ) {
        let client_context = self.context.clone();
        let own_username = self.own_username.clone();
        let Some(client_handle) = self.self_handle.clone() else {
            error!("[client] missing self handle for incoming file connection");
            self.apply_download_status(token, DownloadStatus::Failed, true);
            return;
        };

        if let Err(e) = self.download_executor.execute(move || {
            Self::run_incoming_file_connection(IncomingFileJob {
                peer,
                stream,
                reader,
                token,
                peer_ip,
                peer_port,
                own_username,
                client_context,
                client_handle,
            });
        }) {
            error!("[client] failed to queue incoming file connection: {}", e);
            self.apply_download_status(token, DownloadStatus::Failed, true);
        }
    }

    fn run_incoming_file_connection(mut job: IncomingFileJob) {
        trace!(
            "[client] DownloadFromPeer token: {} peer: {:?}",
            job.token, job.peer
        );

        let download = Self::extract_download_from_reader(
            &mut job.reader,
            &job.client_context,
            &job.peer.username,
            &job.peer_ip,
            job.peer_port,
        );
        let failure_token = download.as_ref().map_or(job.token, |d| d.token);

        let download_peer = DownloadPeer::new(
            format!("{}:direct", job.peer.username),
            job.peer.host.clone(),
            job.peer.port,
            job.token,
            true,
            job.own_username,
        );

        match download_peer.download_file(
            job.client_context.clone(),
            job.client_handle.clone(),
            download,
            Some(job.stream),
        ) {
            Ok((download, filename)) => {
                Self::send_download_status_update(
                    &job.client_handle,
                    download.token,
                    DownloadStatus::Completed,
                    true,
                );
                info!(
                    "Successfully downloaded {} bytes to {}",
                    download.size, filename
                );
            }
            Err(e) => {
                error!(
                    "Failed to download file from {}:{} (token: {}) - Error: {}",
                    job.peer.host, job.peer.port, job.token, e
                );
                Self::send_download_status_update(
                    &job.client_handle,
                    failure_token,
                    DownloadStatus::Failed,
                    true,
                );
            }
        }
    }

    fn extract_download_from_reader(
        reader: &mut MessageReader,
        client_context: &Arc<RwLock<ClientContext>>,
        username: &str,
        peer_ip: &str,
        peer_port: u16,
    ) -> Option<Download> {
        if reader.buffer_len() == 0 {
            return None;
        }
        let buffer = reader.get_buffer();
        let token = Self::parse_token_from_buffer(&buffer, username)?;
        trace!(
            "[listener:{}] got transfer_token: {} from data chunk",
            username, token
        );

        let context = match client_context.read_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[listener] client context lock: {}", e);
                return None;
            }
        };
        let download = context.get_download_by_token(token).cloned();

        if download.is_none() {
            let download_tokens = context.get_download_tokens();
            trace!(
                "[listener:{peer_ip}:{peer_port}] download token not found: {:?}, download tokens: {:?}",
                token, download_tokens
            );
        }

        download
    }

    fn parse_token_from_buffer(buffer: &[u8], username: &str) -> Option<u32> {
        let token_bytes = buffer.get(0..4)?;
        let token = u32::from_le_bytes(token_bytes.try_into().unwrap_or_else(
            |_| {
                panic!(
                    "[listener:{username}] slice with incorrect length, can't extract transfer_token"
                )
            },
        ));
        Some(token)
    }

    fn send_download_status_update(
        client_handle: &ActorHandle<ClientOperation>,
        token: u32,
        status: DownloadStatus,
        notify: bool,
    ) {
        if let Err(e) =
            client_handle.send(ClientOperation::DownloadStatusUpdate {
                token,
                status,
                notify,
            })
        {
            error!("[client] failed to send download status update: {}", e);
        }
    }

    fn apply_download_status(
        &self,
        token: u32,
        status: DownloadStatus,
        notify: bool,
    ) {
        let sender = match self.context.write_safe() {
            Ok(mut ctx) => {
                let sender = ctx
                    .get_download_by_token(token)
                    .map(|download| download.sender.clone());
                ctx.update_download_with_status(token, status.clone());
                sender
            }
            Err(e) => {
                error!("[client] download status write: {}", e);
                None
            }
        };

        if notify && let Some(sender) = sender {
            let _ = sender.send(status);
        }
    }

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
            .filter(|download| {
                download.username == username
                    && matches!(download.status, DownloadStatus::Queued)
            })
            .map(|download| (download.token, download.sender.clone()))
            .collect();

        for (token, sender) in doomed {
            let _ = sender.send(DownloadStatus::Failed);
            context.update_download_with_status(token, DownloadStatus::Failed);
        }
    }

    fn handle_new_peer(&self, new_peer: NewPeer) {
        let peer_exists = match self.context.read_safe() {
            Ok(ctx) => ctx
                .peer_registry
                .as_ref()
                .is_some_and(|r| r.contains(&new_peer.username)),
            Err(e) => {
                error!("[client] NewPeer read: {}", e);
                return;
            }
        };

        if peer_exists {
            debug!("Already connected to {}", new_peer.username);
        } else {
            let send_result = self.context.read_safe().ok().and_then(|ctx| {
                ctx.server_handle.as_ref().map(|s| {
                    s.send(ServerMessage::GetPeerAddress(
                        new_peer.username.clone(),
                    ))
                })
            });
            if let Some(Err(e)) = send_result {
                error!("[client] NewPeer send GetPeerAddress: {}", e);
            }
        }

        let addr = match new_peer.tcp_stream.peer_addr() {
            Ok(a) => a,
            Err(e) => {
                error!("[client] NewPeer peer_addr: {}", e);
                return;
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

        self.connect_or_download(peer, Some(new_peer.tcp_stream), None);
    }

    fn handle_peer_address_response(
        &self,
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    ) {
        debug!(
            "Received peer address for {}: {}:{} (obf_type: {}, obf_port: {})",
            username, host, port, obfuscation_type, obfuscated_port
        );

        let peer_exists = match self.context.read_safe() {
            Ok(ctx) => ctx
                .peer_registry
                .as_ref()
                .is_some_and(|r| r.contains(&username)),
            Err(e) => {
                error!("[client] GetPeerAddressResponse read: {}", e);
                return;
            }
        };

        if peer_exists {
            return;
        }

        let peer = Peer::new(
            username,
            ConnectionType::P,
            host,
            port,
            None,
            0,
            u8::try_from(obfuscation_type).unwrap_or(0),
            obfuscated_port,
        );
        self.connect_or_download(peer, None, None);
    }

    fn update_download_tokens(&self, transfer: Transfer, username: String) {
        let mut context = match self.context.write_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] UpdateDownloadTokens write: {}", e);
                return;
            }
        };

        let download_to_update = context.get_downloads().iter().find_map(|d| {
            if d.username == username && d.filename == transfer.filename {
                Some((d.token, d.clone()))
            } else {
                None
            }
        });

        if let Some((old_token, download)) = download_to_update {
            trace!(
                "[client] UpdateDownloadTokens found {old_token}, transfer: {:?}",
                transfer
            );

            context.add_download(Download {
                username,
                filename: transfer.filename,
                token: transfer.token,
                size: transfer.size,
                download_directory: download.download_directory,
                status: download.status,
                sender: download.sender,
                queue_position: download.queue_position,
                metadata: download.metadata,
            });
            context.remove_download(old_token);
        }
    }
}

impl Actor for ClientActor {
    type Message = ClientOperation;

    fn handle(&mut self, msg: Self::Message) {
        self.handle_operation(msg);
    }

    fn tick(&mut self) {
        self.complete_finished_searches();
        self.complete_expired_broker_connections();
    }

    fn tick_interval(&self) -> Option<Duration> {
        if self.pending_searches.is_empty()
            && self.pending_broker_connections.is_empty()
        {
            None
        } else {
            Some(Duration::from_millis(100))
        }
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
        let actor_system = self.context.read_safe()?.actor_system.clone();
        let client_actor =
            ClientActor::new(self.context.clone(), self.username.clone());
        let client_handle =
            actor_system.spawn_with_handle(client_actor, |actor, handle| {
                actor.set_self_handle(handle);
            });

        {
            let mut ctx = self.context.write_safe()?;
            ctx.client_handle = Some(client_handle.clone());
            ctx.peer_registry = Some(PeerRegistry::new(
                actor_system.clone(),
                client_handle.clone(),
                self.username.clone(),
            ));
        }

        let server_actor = ServerActor::new(
            self.address.clone(),
            client_handle.clone(),
            self.listen_port,
            self.enable_listen,
        );

        let server_handle =
            actor_system.spawn_with_handle(server_actor, |actor, handle| {
                actor.set_self_handle(handle);
            });
        self.server_handle = Some(server_handle.clone());
        self.context.write_safe()?.server_handle = Some(server_handle);

        if self.enable_listen {
            let listener = ListenActor::bind(
                self.listen_port,
                actor_system.clone(),
                client_handle,
            )?;
            actor_system.spawn(listener);
        }

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
            .map_err(SoulseekRs::InvalidMessage)
    }

    /// Ask the server for a peer's address and open a direct control
    /// connection to it.
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
            .map_err(SoulseekRs::InvalidMessage)
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

        let client_handle = self
            .context
            .read_safe()?
            .client_handle
            .clone()
            .ok_or(SoulseekRs::NotConnected)?;
        let (response, receiver) = mpsc::channel();

        client_handle
            .send(ClientOperation::StartSearch {
                query: query.to_string(),
                timeout,
                cancel_flag,
                response,
            })
            .map_err(SoulseekRs::InvalidMessage)?;

        receiver.recv().unwrap_or(Err(SoulseekRs::Timeout))
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

        let registry = {
            let mut context = self.context.write_safe()?;
            context.add_download(download.clone());
            context.peer_registry.clone()
        };

        let peer_registered = registry
            .as_ref()
            .is_some_and(|registry| registry.contains(&username));
        let queued_now = peer_registered
            && registry.as_ref().is_some_and(|registry| {
                registry
                    .queue_upload(&username, download.filename.clone())
                    .is_ok()
            });

        let failed = if peer_registered {
            !queued_now
        } else {
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

    fn connect_to_peer(
        peer: Peer,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
        client_handle: Option<ActorHandle<ClientOperation>>,
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
                    match registry.register_peer(peer_clone, stream, reader) {
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
                let Some(token) = peer.token else {
                    error!(
                        "[client] cannot start file connection for {} without token",
                        peer.username
                    );
                    return;
                };
                let download_peer = DownloadPeer::new(
                    peer.username,
                    peer.host,
                    peer.port,
                    token,
                    false,
                    own_username,
                );
                let Some(client_handle) = client_handle else {
                    error!(
                        "[client] missing client handle for file connection status"
                    );
                    return;
                };

                match download_peer.download_file(
                    client_context,
                    client_handle.clone(),
                    None,
                    stream,
                ) {
                    Ok((download, filename)) => {
                        trace!(
                            "[client] downloaded {} bytes {:?} ",
                            filename, download.size
                        );
                        ClientActor::send_download_status_update(
                            &client_handle,
                            download.token,
                            DownloadStatus::Completed,
                            true,
                        );
                    }
                    Err(e) => {
                        trace!("[client] failed to download: {}", e);
                        ClientActor::send_download_status_update(
                            &client_handle,
                            token,
                            DownloadStatus::Failed,
                            true,
                        );
                    }
                }
            }
            ConnectionType::D => {
                error!("ConnectionType::D not implemented");
            }
        }
    }
}
