use crate::{
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
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    thread::{self, sleep},
};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace};
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
    fn insert_download(
        &mut self,
        username: String,
        filename: String,
        download: Download,
    ) {
        self.downloads
            .entry(format!("{}_{}", username, filename))
            .or_insert(download);
    }

    fn get_download(
        &self,
        username: String,
        filename: String,
    ) -> Option<Download> {
        self.downloads
            .get(&format!("{}_{}", username, filename))
            .cloned()
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
            "ThreadPool initialized with {} threads",
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
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        self.context.lock().unwrap().sender = Some(sender.clone());

        // let client_sender = sender.clone();

        self.server = match Server::new(self.address.clone(), sender) {
            Ok(server) => {
                info!(
                    "Connected to server at {}:{}",
                    server.get_address().get_host(),
                    server.get_address().get_port()
                );

                // thread::spawn(move || {
                //     Listen::start(2234, client_sender.clone());
                // });
                let mut unlocked_context = self.context.lock().unwrap();
                unlocked_context.server_sender =
                    Some(server.get_sender().clone());

                Self::listen_to_client_operations(
                    message_reader,
                    self.context.clone(),
                    self.username.clone(),
                );
                Some(server)
            }
            Err(e) => {
                error!("Error connecting to server: {}", e);
                None
            }
        };
    }

    pub fn login(&self) -> Result<bool> {
        // Attempt to login
        info!("Logging in as {}", self.username);
        if let Some(server) = &self.server {
            let result = server.login(&self.username, &self.password)?;
            if result {
                Ok(true)
            } else {
                Err(SoulseekRs::AuthenticationFailed)
            }
        } else {
            Err(SoulseekRs::NotConnected)
        }
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
        info!("Searching for {}", query);
        if let Some(server) = &self.server {
            let hash = md5::md5(query);
            let token = u32::from_str_radix(&hash[0..5], 16)?;
            server.file_search(token, query);
        } else {
            return Err(SoulseekRs::NotConnected);
        }

        let start = Instant::now();
        loop {
            sleep(Duration::from_millis(500));
            if start.elapsed() >= timeout {
                break;
            }
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

        let hash = md5::md5(&filename);
        let token = u32::from_str_radix(&hash[0..5], 16)?;

        let download_token = DownloadToken {
            username: username.clone(),
            filename: filename.clone(),
            token,
            size,
        };

        let mut context = self.context.lock().unwrap();

        let (tx, rx): (Sender<DownloadStatus>, Receiver<DownloadStatus>) =
            mpsc::channel();

        context.insert_download(
            download_token.username.clone(),
            download_token.filename.clone(),
            Download { channel: tx },
        );

        context
            .download_tokens
            .insert(token, download_token.clone());

        let download_initiated = context
            .peers
            .get(&username)
            .map(|p| p.transfer_request(download_token.clone()))
            .is_some();

        drop(context);

        if !download_initiated {
            return Ok(DownloadResult {
                filename,
                username,
                status: DownloadStatus::Failed,
                elapsed_time: start_time.elapsed(),
            });
        }

        let download_status = rx.recv().unwrap();

        Ok(DownloadResult {
            filename,
            username,
            status: download_status,
            elapsed_time: start_time.elapsed(),
        })
    }

    fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || loop {
            if let Ok(operation) = reader.recv() {
                match operation {
                    ClientOperation::ConnectToPeer(peer) => {
                        trace!(
                            "[client] Connecting to peer: {:?}",
                            peer.username
                        );
                        Self::connect_to_peer(
                            peer,
                            client_context.clone(),
                            own_username.clone(),
                            None,
                        )
                    }
                    ClientOperation::SearchResult(file_search) => {
                        client_context
                            .lock()
                            .unwrap()
                            .search_results
                            .push(file_search);
                    }
                    ClientOperation::PeerDisconnected(username) => {
                        let mut context = client_context.lock().unwrap();
                        if let Some(peer) = context.peers.remove(&username) {
                            drop(peer); // Explicitly drop to trigger cleanup
                        }
                    }
                    ClientOperation::PierceFireWall(peer) => {
                        Self::pierce_firewall(
                            peer,
                            client_context.clone(),
                            own_username.clone(),
                        );
                    }
                    ClientOperation::DownloadFromPeer(token, peer) => {
                        let Some(download) =
                            Self::get_download_token(&client_context, token)
                        else {
                            error!(
                                "Can't find download with token {:?}",
                                token
                            );
                            continue;
                        };

                        let filename_path = download.filename.clone();
                        let Some(filename) =
                            Self::extract_filename(&filename_path)
                        else {
                            error!(
                                "Cant find filename to save download: {:?}",
                                filename_path
                            );
                            continue;
                        };

                        Self::spawn_download_task(
                            download,
                            peer,
                            filename.to_string(),
                            client_context.clone(),
                            own_username.clone(),
                        );
                    }
                    ClientOperation::NewPeer(new_peer) => {
                        if client_context
                            .lock()
                            .unwrap()
                            .peers
                            .contains_key(&new_peer.username)
                        {
                            debug!(
                                "Already connected to {}",
                                new_peer.username
                            );
                        } else if let Some(server_sender) =
                            &client_context.lock().unwrap().server_sender
                        {
                            server_sender
                                .send(ServerOperation::GetPeerAddress(
                                    new_peer.username.clone(),
                                ))
                                .unwrap();
                        }

                        let addr = new_peer.tcp_stream.peer_addr().unwrap();
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
                            username, host, port, obfuscation_type, obfuscated_port
                        );

                        match client_context
                            .lock()
                            .unwrap()
                            .peers
                            .get(&username)
                        {
                            Some(_peer) => {
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
                            None => {
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
                                Self::connect_to_peer(
                                    peer,
                                    client_context.clone(),
                                    own_username.clone(),
                                    None,
                                );
                            }
                        }
                    }
                    ClientOperation::UpdateDownloadTokens(
                        transfer,
                        username,
                    ) => {
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
            }
        });
    }

    fn connect_to_peer(
        peer: Peer,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
        stream: Option<TcpStream>,
    ) {
        let client_context2 = client_context.clone();
        let unlocked_context = client_context.lock().unwrap();

        trace!("[client] connect_to_peer: {}", peer.username);
        if let Some(sender) = unlocked_context.sender.clone() {
            let peer_clone = peer.clone();
            let sender_clone = sender;
            unlocked_context.thread_pool.execute(move || {
                trace!("[client] connecting to {}, with connection_type: {}", peer.username, peer.connection_type);
                match peer.connection_type {
                    ConnectionType::P => {
                        let default_peer =
                            DefaultPeer::new(peer_clone, sender_clone);

                        let connect_result = match stream {
                            Some(s) => default_peer.connect_with_socket(s),
                            None => default_peer.connect()
                        };

                        match connect_result {
                            Ok(p) => {
                                trace!("[client] connected to: {}", peer.username);
                                client_context2.lock().unwrap().peers.insert(peer.username, p);
                            }
                            Err(e) => {
                                trace!(
                                    "[client] Can't connect to {:?} {:?}:{:?} {:?} - {:?}",
                                    peer.username,
                                    peer.host,
                                    peer.port,
                                    peer.connection_type,
                                    e
                                );
                            }
                        }
                    }

                    ConnectionType::F => {
                        trace!("[client] downloading from: {}", peer.username);
                        let download_peer = DownloadPeer::new(
                            peer.username.clone(),
                            peer.host,
                            peer.port,
                            peer.token.unwrap(),
                            false,
                            own_username.clone(),
                        );

                        match download_peer
                            .download_file(
                                client_context2.clone(),
                                None,
                                None
                            ) {
                                Ok((bytes, filename)) => {
                                    if let Some(download) =
                                        client_context2.lock().unwrap().get_download(
                                            peer.username.clone(),
                                            filename.to_string())
                                    {
                                        download.channel.send(DownloadStatus::Completed).unwrap();

                                        info!(
                                            "[client] Successfully downloaded {} bytes to /tmp/{}",
                                            bytes, filename.to_string(),
                                        );
                                    } else {
                                        error!(
                                            "[client] No download found for {} with filename {}",
                                            peer.username, filename
                                        );
                                    }

                                }
                                Err(e) => {
                                    trace!("[client] failed to download: {}", e);
                                }
                            }
                    }
                    ConnectionType::D => {
                        error!("[client] ConnectionType::D not implemented")
                    }
                }
            });
        } else {
            debug!("[client] Peer already connected: {}", peer.username);
        }
    }
    fn pierce_firewall(
        peer: Peer,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        debug!("Piercing firewall for peer: {:?}", peer);

        let context = client_context.lock().unwrap();
        if let Some(server_sender) = &context.server_sender {
            if let Some(token) = peer.token {
                match server_sender.send(ServerOperation::PierceFirewall(token))
                {
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
        match result {
            Ok((bytes, downloaded_filename)) => {
                if let Some(download) =
                    client_context.lock().unwrap().get_download(
                        peer_username.to_string(),
                        filename.to_string(),
                    )
                {
                    download.channel.send(DownloadStatus::Completed).unwrap();

                    info!(
                        "Successfully downloaded {} bytes to /tmp/{}",
                        bytes, downloaded_filename
                    );
                } else {
                    error!(
                        "No download found for {} with filename {}",
                        peer_username, filename
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to download file '{}' from {}:{} (token: {}) - Error: {}",
                    filename, peer.host, peer.port, token, e
                );
                if let Some(download) =
                    client_context.lock().unwrap().get_download(
                        peer_username.to_string(),
                        filename.to_string(),
                    )
                {
                    download.channel.send(DownloadStatus::Failed).unwrap();
                } else {
                    error!(
                        "No download found for {} with filename {}",
                        peer_username, filename
                    );
                }
            }
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
}
