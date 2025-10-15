use crate::{
    debug, error, info, trace,
    error::{Result, SoulseekRs},
    peer::{ConnectionType, DefaultPeer, DownloadPeer, NewPeer, Peer},
    server::{PeerAddress, Server, ServerOperation},
    types::{Download, DownloadToken, FileSearchResult},
    utils::{md5, thread_pool::ThreadPool},
    DownloadResult, DownloadStatus, Transfer,
};
use std::{
    collections::HashMap,
    net::TcpStream,
    sync::{mpsc::{self, Receiver, Sender}, Arc, Mutex},
    thread::{self, sleep},
    time::{Duration, Instant},
};
const MAX_THREADS: usize = 50;
pub enum ClientOperation {
    NewPeer(NewPeer),
    ConnectToPeer(Peer),
    SearchResult(FileSearchResult),
    PeerDisconnected(String),
    PierceFireWall(Peer),
    DownloadFromPeer(u32, Peer),
    UpdateDownloadTokens(Transfer, String),
    GetPeerAddressResponse {
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    },
}
pub struct ClientContext {
    peers: HashMap<String, DefaultPeer>,
    sender: Option<Sender<ClientOperation>>,
    server_sender: Option<Sender<crate::server::ServerOperation>>,
    search_results: Vec<FileSearchResult>,
    pub download_tokens: HashMap<u32, DownloadToken>,
    pub downloads: HashMap<String, Download>,
    thread_pool: ThreadPool,
}
impl Default for ClientContext {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            sender: None,
            server_sender: None,
            search_results: Vec::new(),
            download_tokens: HashMap::new(),
            downloads: HashMap::new(),
            thread_pool: ThreadPool::new(MAX_THREADS),
        }
    }
}
impl ClientContext {
    fn download_key(username: &str, filename: &str) -> String {
        format!("{}_{}", username, filename)
    }

    fn insert_download(
        &mut self,
        username: String,
        filename: String,
        download: Download,
    ) {
        self.downloads
            .entry(Self::download_key(&username, &filename))
            .or_insert(download);
    }

    fn get_download(
        &self,
        username: &str,
        filename: &str,
    ) -> Option<Download> {
        self.downloads
            .get(&Self::download_key(username, filename))
            .cloned()
    }

    fn send_download_status(
        &self,
        username: &str,
        filename: &str,
        status: DownloadStatus,
    ) -> Result<()> {
        self.get_download(username, filename)
            .ok_or(SoulseekRs::ParseError(
                format!("No download found for {} with filename {}", username, filename)
            ))?
            .channel
            .send(status)
            .map_err(|_| SoulseekRs::NetworkError(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Channel send failed")
            ))
    }
}
pub struct Client {
    address: PeerAddress,
    username: String,
    password: String,
    server: Option<Server>,
    context: Arc<Mutex<ClientContext>>,
}

impl Client {
    pub fn new(
        address: PeerAddress,
        username: String,
        password: String,
    ) -> Self {
        crate::utils::logger::init();
        let context = Arc::new(Mutex::new(ClientContext::default()));
        debug!(
            "[client] ThreadPool initialized with {} threads",
            context.lock().unwrap().thread_pool.thread_count()
        );
        Self {
            address,
            username,
            password,
            server: None,
            context,
        }
    }

    pub fn connect(&mut self) {
        let (sender, message_reader) = mpsc::channel();
        self.context.lock().unwrap().sender = Some(sender.clone());

        // let client_sender = sender.clone();

        self.server = Server::new(self.address.clone(), sender)
            .map(|server| {
                info!(
                    "[client] Connected to server at {}:{}",
                    server.get_address().get_host(),
                    server.get_address().get_port()
                );

                // thread::spawn(move || {
                //     Listen::start(2234, client_sender.clone());
                // });

                self.context.lock().unwrap().server_sender = Some(server.get_sender().clone());
                Self::listen_to_client_operations(
                    message_reader,
                    self.context.clone(),
                    self.username.clone(),
                );
                server
            })
            .map_err(|e| {
                error!("[client] Error connecting to server: {}", e);
                e
            })
            .ok();
    }

    pub fn login(&self) -> Result<bool> {
        info!("[client] Logging in as {}", self.username);
        let server = self.server.as_ref().ok_or(SoulseekRs::NotConnected)?;

        server.login(&self.username, &self.password)?
            .then(|| Ok(true))
            .unwrap_or(Err(SoulseekRs::AuthenticationFailed))
    }

    #[allow(dead_code)]
    pub fn remove_peer(&self, username: &str) {
        let mut context = self.context.lock().unwrap();
        if let Some(peer) = context.peers.remove(username) {
            drop(peer);
        }
    }
    pub fn search(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Vec<FileSearchResult>> {
        info!("[client] Searching for {}", query);
        let server = self.server.as_ref().ok_or(SoulseekRs::NotConnected)?;

        let hash = md5::md5(query);
        let token = u32::from_str_radix(&hash[0..5], 16)?;
        server.file_search(token, query);

        let start = Instant::now();
        while start.elapsed() < timeout {
            sleep(Duration::from_millis(500));
        }
        Ok(self.context.lock().unwrap().search_results.clone())
    }

    pub fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
    ) -> Result<crate::types::DownloadResult> {
        let start_time = Instant::now();
        let token = u32::from_str_radix(&md5::md5(&filename)[0..5], 16)?;
        let (tx, rx) = mpsc::channel();

        let download_token = DownloadToken {
            username: username.clone(),
            filename: filename.clone(),
            token,
            size,
        };

        let download_initiated = {
            let mut context = self.context.lock().unwrap();
            context.insert_download(
                username.clone(),
                filename.clone(),
                Download { channel: tx },
            );
            context.download_tokens.insert(token, download_token.clone());

            context.peers
                .get(&username)
                .map(|p| p.transfer_request(download_token))
                .is_some()
        };

        let status = if download_initiated {
            rx.recv().unwrap_or(DownloadStatus::Failed)
        } else {
            DownloadStatus::Failed
        };

        Ok(DownloadResult {
            filename,
            username,
            status,
            elapsed_time: start_time.elapsed(),
        })
    }

    fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || loop {
            let Ok(operation) = reader.recv() else { continue };

            match operation {
                ClientOperation::ConnectToPeer(peer) => {
                    trace!("[client] Connecting to peer: {:?}", peer.username);
                    Self::connect_to_peer(peer, client_context.clone(), own_username.clone(), None)
                }
                ClientOperation::SearchResult(file_search) => {
                    client_context.lock().unwrap().search_results.push(file_search);
                }
                ClientOperation::PeerDisconnected(username) => {
                    client_context.lock().unwrap().peers.remove(&username).map(drop);
                }
                ClientOperation::PierceFireWall(peer) => {
                    Self::pierce_firewall(
                        peer,
                        client_context.clone(),
                        own_username.clone(),
                    );
                }
                ClientOperation::DownloadFromPeer(token, peer) => {
                    Self::handle_download_from_peer(
                        token,
                        peer,
                        &client_context,
                        &own_username,
                    );
                }
                ClientOperation::NewPeer(new_peer) => {
                    Self::handle_new_peer(new_peer, &client_context, &own_username);
                }
                ClientOperation::GetPeerAddressResponse {
                    username, host, port, obfuscation_type, obfuscated_port,
                } => {
                    debug!(
                        "[client] Received peer address for {}: {}:{} (obf_type: {}, obf_port: {})",
                        username, host, port, obfuscation_type, obfuscated_port
                    );

                    if !client_context.lock().unwrap().peers.contains_key(&username) {
                        let peer = Peer::new(
                            username,
                            ConnectionType::P,
                            host,
                            port,
                            None,
                            0,
                            obfuscation_type as u8,
                            obfuscated_port as u8,
                        );
                        Self::connect_to_peer(peer, client_context.clone(), own_username.clone(), None);
                    }
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
                }
                ClientOperation::UpdateDownloadTokens(transfer, username) => {
                    client_context.lock().unwrap().download_tokens.insert(
                        transfer.token,
                        DownloadToken {
                            username,
                            filename: transfer.filename,
                            token: transfer.token,
                            size: transfer.size,
                        },
                    );
                }
            }
        });
    }

    fn connect_to_peer(
        peer: Peer,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
        stream: Option<TcpStream>,
    ) {
        trace!("[client] connect_to_peer: {}", peer.username);

        let Some(sender) = client_context.lock().unwrap().sender.clone() else {
            debug!("[client] Peer already connected: {}", peer.username);
            return;
        };

        let context_clone = client_context.clone();
        client_context.lock().unwrap().thread_pool.execute(move || {
            trace!("[client] connecting to {}, with connection_type: {}", peer.username, peer.connection_type);

            match peer.connection_type {
                ConnectionType::P => Self::handle_peer_connection(peer, sender, context_clone, stream),
                ConnectionType::F => Self::handle_file_download(peer, context_clone, own_username),
                ConnectionType::D => error!("[client] ConnectionType::D not implemented"),
            }
        });
    }

    fn handle_peer_connection(
        peer: Peer,
        sender: Sender<ClientOperation>,
        context: Arc<Mutex<ClientContext>>,
        stream: Option<TcpStream>,
    ) {
        let default_peer = DefaultPeer::new(peer.clone(), sender);
        let connect_result = match stream {
            Some(s) => default_peer.connect_with_socket(s),
            None => default_peer.connect(),
        };

        match connect_result {
            Ok(p) => {
                trace!("[client] connected to: {}", peer.username);
                context.lock().unwrap().peers.insert(peer.username, p);
            }
            Err(e) => {
                trace!(
                    "[client] Can't connect to {:?} {:?}:{:?} {:?} - {:?}",
                    peer.username, peer.host, peer.port, peer.connection_type, e
                );
            }
        }
    }

    fn handle_file_download(
        peer: Peer,
        context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        trace!("[client] downloading from: {}", peer.username);
        let download_peer = DownloadPeer::new(
            peer.username.clone(),
            peer.host,
            peer.port,
            peer.token.unwrap_or(0),
            false,
            own_username,
        );

        match download_peer.download_file(context.clone(), None, None) {
            Ok((bytes, filename)) => {
                let result = context.lock().unwrap().send_download_status(
                    &peer.username,
                    &filename,
                    DownloadStatus::Completed,
                );

                if result.is_ok() {
                    info!("[client] Successfully downloaded {} bytes to /tmp/{}", bytes, filename);
                } else {
                    error!("[client] No download found for {} with filename {}", peer.username, filename);
                }
            }
            Err(e) => trace!("[client] failed to download: {}", e),
        }
    }
    fn pierce_firewall(
        peer: Peer,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        debug!("[client] Piercing firewall for peer: {:?}", peer);

        let context = client_context.lock().unwrap();
        if let (Some(server_sender), Some(token)) = (&context.server_sender, peer.token) {
            if let Err(e) = server_sender.send(ServerOperation::PierceFirewall(token)) {
                error!("[client] Failed to send PierceFirewall message: {}", e);
            }
        } else {
            error!("[client] No {} available for PierceFirewall",
                if peer.token.is_none() { "token" } else { "server sender" }
            );
        }
        drop(context);

        Self::connect_to_peer(peer, client_context, own_username, None);
    }

    fn get_download_token(
        client_context: &Arc<Mutex<ClientContext>>,
        token: u32,
    ) -> Option<DownloadToken> {
        let context = client_context.lock().unwrap();
        context.download_tokens.get(&token).cloned()
    }

    fn extract_filename(filename: &str) -> Option<&str> {
        filename.split('\\').last()
    }

    fn handle_download_result(
        client_context: &Arc<Mutex<ClientContext>>,
        peer_username: &str,
        filename: &str,
        result: std::result::Result<(usize, String), std::io::Error>,
        peer: &Peer,
        token: u32,
    ) {
        let (status, log_msg) = match result {
            Ok((bytes, downloaded_filename)) => {
                let msg = format!("Successfully downloaded {} bytes to /tmp/{}", bytes, downloaded_filename);
                (DownloadStatus::Completed, msg)
            }
            Err(e) => {
                let msg = format!(
                    "Failed to download file '{}' from {}:{} (token: {}) - Error: {}",
                    filename, peer.host, peer.port, token, e
                );
                (DownloadStatus::Failed, msg)
            }
        };

        let send_result = client_context.lock().unwrap()
            .send_download_status(peer_username, filename, status);

        match (status, send_result) {
            (DownloadStatus::Completed, Ok(_)) => info!("[client] {}", log_msg),
            (DownloadStatus::Failed, _) => error!("[client] {}", log_msg),
            (_, Err(_)) => error!("[client] No download found for {} with filename {}", peer_username, filename),
            _ => {}
        }
    }

    fn spawn_download_task(
        download: DownloadToken,
        peer: Peer,
        filename: String,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || {
            let download_peer = DownloadPeer::new(
                download.username.clone(),
                peer.host.clone(),
                peer.port,
                download.token,
                false,
                own_username,
            );

            let result = download_peer.download_file(
                client_context.clone(),
                Some(download.size as usize),
                Some(format!("/tmp/{}", filename)),
            );

            Self::handle_download_result(
                &client_context,
                &peer.username,
                &filename,
                result,
                &peer,
                download.token,
            );
        });
    }

    fn handle_new_peer(
        new_peer: NewPeer,
        client_context: &Arc<Mutex<ClientContext>>,
        own_username: &str,
    ) {
        {
            let context = client_context.lock().unwrap();
            if context.peers.contains_key(&new_peer.username) {
                debug!("[client] Already connected to {}", new_peer.username);
            } else if let Some(server_sender) = &context.server_sender {
                let _ = server_sender.send(ServerOperation::GetPeerAddress(new_peer.username.clone()));
            }
        }

        let addr = new_peer.tcp_stream.peer_addr().unwrap();
        let peer = Peer {
            username: new_peer.username,
            connection_type: new_peer.connection_type,
            host: addr.ip().to_string(),
            port: addr.port().into(),
            token: Some(new_peer.token),
            privileged: None,
            obfuscated_port: None,
            unknown: None,
        };

        Self::connect_to_peer(
            peer,
            client_context.clone(),
            own_username.to_string(),
            Some(new_peer.tcp_stream),
        );
    }

    fn handle_download_from_peer(
        token: u32,
        peer: Peer,
        client_context: &Arc<Mutex<ClientContext>>,
        own_username: &str,
    ) {
        let Some(download) = Self::get_download_token(client_context, token) else {
            error!("[client] Can't find download with token {:?}", token);
            return;
        };

        let filename_path = download.filename.clone();
        let Some(filename) = Self::extract_filename(&filename_path) else {
            error!("[client] Can't find filename to save download: {:?}", filename_path);
            return;
        };

        Self::spawn_download_task(
            download,
            peer,
            filename.to_string(),
            client_context.clone(),
            own_username.to_string(),
        );
    }
}
