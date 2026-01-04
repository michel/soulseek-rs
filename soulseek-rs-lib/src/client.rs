use crate::actor::ActorHandle;
use crate::actor::server_actor::{PeerAddress, ServerActor, ServerMessage};
use crate::types::DownloadStatus;
use crate::utils::logger;
use crate::{
    Transfer,
    actor::{ActorSystem, peer_registry::PeerRegistry},
    error::{Result, SoulseekRs},
    peer::{ConnectionType, DownloadPeer, NewPeer, Peer, listen::Listen},
    types::{Download, Search, SearchResult},
    utils::{md5, thread_pool::ThreadPool},
};
use std::{
    collections::HashMap,
    net::TcpStream,
    sync::{
        RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
    thread::{self, sleep},
};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace, warn};
const DEFALT_LISTEN_PORT: u32 = 2234;

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub username: String,
    pub password: String,
    pub server_address: PeerAddress,
    pub enable_listen: bool,
    pub listen_port: u32,
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
            listen_port: DEFALT_LISTEN_PORT,
        }
    }
}

#[derive(Debug)]

pub enum ClientOperation {
    NewPeer(NewPeer),
    ConnectToPeer(Peer),
    SearchResult(SearchResult),
    PeerDisconnected(String, Option<SoulseekRs>),
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
    SetServerSender(Sender<ServerMessage>),
}
pub struct ClientContext {
    pub peer_registry: Option<PeerRegistry>,
    sender: Option<Sender<ClientOperation>>,
    server_sender: Option<Sender<ServerMessage>>,
    searches: HashMap<String, Search>,
    downloads: Vec<Download>,
    actor_system: Arc<ActorSystem>,
}
impl Default for ClientContext {
    fn default() -> Self {
        Self::new()
    }
}
impl ClientContext {
    pub fn add_download(&mut self, download: Download) {
        self.downloads.push(download);
    }
    pub fn remove_download(&mut self, token: u32) {
        self.downloads.retain(|d| d.token != token);
    }
    pub fn get_download_by_token(&self, token: u32) -> Option<&Download> {
        self.downloads.iter().find(|d| d.token == token)
    }

    pub fn get_download_by_token_mut(
        &mut self,
        token: u32,
    ) -> Option<&mut Download> {
        self.downloads.iter_mut().find(|d| d.token == token)
    }
    pub fn get_download_tokens(&self) -> Vec<u32> {
        self.downloads.iter().map(|d| d.token).collect()
    }
    pub fn get_downloads(&self) -> &Vec<Download> {
        &self.downloads
    }

    pub fn update_download_with_status(
        &mut self,
        token: u32,
        status: DownloadStatus,
    ) {
        if let Some(download) = self.get_download_by_token_mut(token) {
            download.status = status;
        }
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
    };
    context.add_download(download);
    assert!(context.get_download_by_token(123).is_some());
    assert_eq!(context.get_download_tokens(), vec![123]);
    assert_eq!(context.get_downloads().len(), 1);
    if let Some(download) = context.get_download_by_token_mut(token) {
        assert_eq!(download.token, token);
        download.token = new_token
    }
    assert!(context.get_download_by_token(new_token).is_some());
    assert_eq!(context.get_download_tokens(), vec![new_token]);
    context.remove_download(new_token);
    assert_eq!(context.get_downloads().len(), 0);
    assert!(context.get_download_by_token(1234).is_none());
}

impl ClientContext {
    #[must_use]
    pub fn new() -> Self {
        let max_threads = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);

        let thread_pool = Arc::new(ThreadPool::new(max_threads));
        let actor_system = Arc::new(ActorSystem::new(thread_pool.clone()));

        Self {
            peer_registry: None,
            sender: None,
            server_sender: None,
            searches: HashMap::new(),
            downloads: Vec::new(),
            actor_system,
        }
    }
}
pub struct Client {
    enable_listen: bool,
    listen_port: u32,
    address: PeerAddress,
    username: String,
    password: String,
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

    pub fn with_settings(settings: ClientSettings) -> Self {
        logger::init();
        Self {
            enable_listen: settings.enable_listen,
            listen_port: settings.listen_port,
            address: settings.server_address,
            username: settings.username,
            password: settings.password,
            context: Arc::new(RwLock::new(ClientContext::new())),
            server_handle: None,
        }
    }

    pub fn connect(&mut self) {
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        let mut ctx = self.context.write().unwrap();
        ctx.sender = Some(sender.clone());
        let peer_registry =
            PeerRegistry::new(ctx.actor_system.clone(), sender.clone());
        ctx.peer_registry = Some(peer_registry);

        let listen_sender = sender.clone();

        let server_actor = ServerActor::new(
            self.address.clone(),
            sender,
            self.listen_port,
            self.enable_listen,
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

    #[allow(dead_code)]
    pub fn remove_peer(&self, username: &str) {
        let context = self.context.read().unwrap();
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
            self.context.write().unwrap().searches.insert(
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

    pub fn get_search_results_count(&self, search_key: &str) -> usize {
        self.context
            .read()
            .unwrap()
            .searches
            .get(search_key)
            .map(|s| s.results.len())
            .unwrap_or(0)
    }

    pub fn get_search_results(&self, search_key: &str) -> Vec<SearchResult> {
        self.context
            .read()
            .unwrap()
            .searches
            .get(search_key)
            .map(|s| s.results.clone())
            .unwrap_or_default()
    }

    /// Non-blocking variant that returns None if the lock is unavailable
    pub fn try_get_search_results(
        &self,
        search_key: &str,
    ) -> Option<Vec<SearchResult>> {
        self.context.try_read().ok().and_then(|ctx| {
            ctx.searches.get(search_key).map(|s| s.results.clone())
        })
    }

    pub fn get_all_searches(&self) -> HashMap<String, Search> {
        self.context.read().unwrap().searches.clone()
    }

    pub fn get_all_downloads(&self) -> Vec<Download> {
        self.context.read().unwrap().get_downloads().clone()
    }

    pub fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
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
            filename: filename.clone(),
            token,
            size,
            download_directory,
            status: DownloadStatus::Queued,
            sender: download_sender,
        };

        let mut context = self.context.write().unwrap();
        context.add_download(download.clone());

        let download_initiated =
            if let Some(ref registry) = context.peer_registry {
                registry
                    .queue_upload(&username, download.filename.clone())
                    .is_ok()
            } else {
                false
            };

        drop(context);

        if !download_initiated {
            let _ = download.sender.send(DownloadStatus::Failed);
            self.context
                .write()
                .unwrap()
                .update_download_with_status(token, DownloadStatus::Failed);
        }

        Ok((download, download_receiver))
    }

    fn process_failed_uploads(
        client_context: Arc<RwLock<ClientContext>>,
        username: &str,
        filename: Option<&str>,
    ) {
        let context = client_context.read().unwrap();
        let failed_downloads: Vec<_> = context
            .get_downloads()
            .iter()
            .filter(|download| {
                download.username == username
                    && (filename.is_none_or(|f| download.filename == *f))
            })
            .map(|download| {
                let _ = download.sender.send(DownloadStatus::Failed);
                download.token
            })
            .collect();
        drop(context);

        failed_downloads.iter().for_each(|token| {
            let mut context = client_context.write().unwrap();
            context.update_download_with_status(*token, DownloadStatus::Failed);
            context.remove_download(*token);
        });
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
                                let mut context =
                                    client_context.write().unwrap();
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
                                username,
                                error,
                            ) => {
                                let context = client_context.read().unwrap();
                                if let Some(ref registry) =
                                    context.peer_registry
                                    && let Some(handle) =
                                        registry.remove_peer(&username)
                                {
                                    let _ = handle.stop();
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
                                let maybe_download = {
                                    let client_context =
                                        client_context.read().unwrap();
                                    client_context
                                        .get_download_by_token(token)
                                        .cloned()
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
                                                            client_context_clone.write().unwrap().update_download_with_status(download.token, DownloadStatus::Completed);
                                                            info!(
                                                                "Successfully downloaded {} bytes to {}",
                                                                download.size,
                                                                filename
                                                            );
                                                        }
                                                        Err(e) => {
                                                            let _ = download.sender.send(DownloadStatus::Failed);
                                                            client_context_clone.write().unwrap().update_download_with_status(download.token, DownloadStatus::Failed);
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
                                };
                            }
                            ClientOperation::NewPeer(new_peer) => {
                                let peer_exists = client_context
                                    .read()
                                    .unwrap()
                                    .peer_registry
                                    .as_ref()
                                    .map(|r| r.contains(&new_peer.username))
                                    .unwrap_or(false);

                                if peer_exists {
                                    debug!(
                                        "Already connected to {}",
                                        new_peer.username
                                    );
                                } else if let Some(server_sender) =
                                    &client_context
                                        .read()
                                        .unwrap()
                                        .server_sender
                                {
                                    server_sender
                                        .send(ServerMessage::GetPeerAddress(
                                            new_peer.username.clone(),
                                        ))
                                        .unwrap();
                                }

                                let addr =
                                    new_peer.tcp_stream.peer_addr().unwrap();
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

                                let peer_exists = client_context
                                    .read()
                                    .unwrap()
                                    .peer_registry
                                    .as_ref()
                                    .map(|r| r.contains(&username))
                                    .unwrap_or(false);

                                if peer_exists {
                                    // don't know if i should update? and or reconnect the peer
                                    // debug!(
                                    //     "existing peer: {:?}, new peer details:
                                    //     username: {},
                                    //     host: {},
                                    //     port: {}
                                    //     obfuscation_type: {}
                                    //     obfuscated_port: {}",
                                    //     peer,
                                    //     username,
                                    //     host,
                                    //     port,
                                    //     obfuscation_type,
                                    //     obfuscated_port,
                                    // );
                                } else {
                                    let peer = Peer::new(
                                        username,
                                        ConnectionType::P,
                                        host,
                                        port,
                                        None,
                                        0,
                                        obfuscation_type.try_into().unwrap(),
                                        obfuscated_port.try_into().unwrap(),
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
                                let mut context =
                                    client_context.write().unwrap();

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
                            ClientOperation::SetServerSender(sender) => {
                                client_context.write().unwrap().server_sender =
                                    Some(sender);
                                debug!("[client] Server sender initialized");
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
        let client_context = client_context.clone();

        let peer_clone = peer.clone();
        trace!(
            "[client] connecting to {}, with connection_type: {}, and token {:?}",
            peer.username, peer.connection_type, peer.token
        );
        match peer.connection_type {
            ConnectionType::P => {
                let username = peer.username.clone();

                let context = client_context.read().unwrap();
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
                    own_username.clone(),
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
                        client_context
                            .write()
                            .unwrap()
                            .update_download_with_status(
                                download.token,
                                DownloadStatus::Completed,
                            );
                    }
                    Err(e) => {
                        trace!("[client] failed to download: {}", e);
                    }
                }
            }
            ConnectionType::D => {
                error!("ConnectionType::D not implemented")
            }
        }
    }
    fn pierce_firewall(
        peer: Peer,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        debug!("Piercing firewall for peer: {:?}", peer);

        let context = client_context.read().unwrap();
        if let Some(server_sender) = &context.server_sender {
            if let Some(token) = peer.token {
                match server_sender.send(ServerMessage::PierceFirewall(token)) {
                    Ok(_) => (),
                    Err(e) => {
                        error!("Failed to send PierceFirewall message: {}", e)
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
