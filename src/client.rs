use crate::types::{DownloadResult, DownloadStatus};
use crate::{
    error::{Result, SoulseekRs},
    peer::{
        listen::Listen, ConnectionType, DefaultPeer, DownloadPeer, NewPeer,
        Peer,
    },
    server::{PeerAddress, Server, ServerOperation},
    types::{Download, FileSearchResult},
    utils::{md5, thread_pool::ThreadPool},
    Transfer,
};
use std::{
    collections::HashMap,
    net::TcpStream,
    sync::{
        mpsc::{Receiver, Sender},
        RwLock,
    },
    thread::{self, sleep},
};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace};
const MAX_THREADS: usize = 50;
const DEFALT_LISTEN_PORT: u32 = 2234;

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
    pub download_tokens: HashMap<u32, Download>,
    thread_pool: ThreadPool,
}
impl Default for ClientContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientContext {
    #[must_use]
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            sender: None,
            server_sender: None,
            search_results: Vec::new(),
            download_tokens: HashMap::new(),
            thread_pool: ThreadPool::new(MAX_THREADS),
        }
    }
}
pub struct Client {
    listen_port: u32,
    address: PeerAddress,
    username: String,
    password: String,
    server: Option<Server>,
    context: Arc<RwLock<ClientContext>>,
}

impl Client {
    pub fn new(
        address: PeerAddress,
        username: String,
        password: String,
        listen_port: Option<u32>,
    ) -> Self {
        crate::utils::logger::init();
        let context = Arc::new(RwLock::new(ClientContext::new()));
        debug!(
            "ThreadPool initialized with {} threads",
            context.read().unwrap().thread_pool.thread_count()
        );
        Self {
            listen_port: listen_port.unwrap_or(DEFALT_LISTEN_PORT),
            address,
            username,
            password,
            server: None,
            context,
        }
    }
    pub fn with_defaults(
        address: PeerAddress,
        username: String,
        password: String,
    ) -> Self {
        Self::new(address, username, password, None)
    }

    pub fn connect(&mut self) {
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        let listen_port = self.listen_port;
        self.context.write().unwrap().sender = Some(sender.clone());
        let context = self.context.clone();
        let own_username = self.username.clone();

        let client_sender = sender.clone();

        self.server =
            match Server::new(self.address.clone(), sender, self.listen_port) {
                Ok(server) => {
                    info!(
                        "Connected to server at {}:{}",
                        server.get_address().get_host(),
                        server.get_address().get_port()
                    );

                    // thread::spawn(move || {
                    //     Listen::start(
                    //         listen_port,
                    //         client_sender.clone(),
                    //         context.clone(),
                    //         own_username,
                    //     );
                    // });
                    let mut unlocked_context = self.context.write().unwrap();
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
        let mut context = self.context.write().unwrap();
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
        Ok(self.context.read().unwrap().search_results.clone())
    }

    pub fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
    ) -> Result<crate::types::DownloadResult> {
        let start_time = Instant::now();

        let hash = md5::md5(&filename);
        let token = u32::from_str_radix(&hash[0..5], 16)?;

        let (download_sender, download_receivier): (
            Sender<DownloadStatus>,
            Receiver<DownloadStatus>,
        ) = mpsc::channel();

        let download = Download {
            username: username.clone(),
            filename: filename.clone(),
            token,
            size,
            download_directory,
            sender: download_sender,
        };

        let mut context = self.context.write().unwrap();
        context.download_tokens.insert(token, download.clone());
        let download_initiated = context
            .peers
            .get(&username)
            .map(|p| p.queue_upload(download.filename))
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

        let status = download_receivier.recv().unwrap();

        Ok(DownloadResult {
            filename,
            username,
            status,
            elapsed_time: start_time.elapsed(),
        })
    }

    fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || loop {
            if let Ok(operation) = reader.recv() {
                match operation {
                    ClientOperation::ConnectToPeer(peer) => {
                        Self::connect_to_peer(
                            peer,
                            client_context.clone(),
                            own_username.clone(),
                            None,
                        );
                    }
                    ClientOperation::SearchResult(file_search) => {
                        trace!("[client] SearchResult {:?}", file_search);
                        client_context
                            .write()
                            .unwrap()
                            .search_results
                            .push(file_search.clone());
                    }
                    ClientOperation::PeerDisconnected(username) => {
                        let mut context = client_context.write().unwrap();
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
                        let maybe_download = {
                            let client_context = client_context.read().unwrap();
                            client_context.download_tokens.get(&token).cloned()
                        };
                        let own_username = own_username.clone();
                        let client_context_clone = client_context.clone();

                        trace!(
                            "[client] DownloadFromPeer token: {} peer: {:?}",
                            token,
                            peer
                        );
                        return match maybe_download {
                            Some(download) => {
                                thread::spawn(move || {
                                    let download_peer = DownloadPeer::new(
                                        download.username.clone(),
                                        peer.host.clone(),
                                        peer.port,
                                        token,
                                        false,
                                        own_username,
                                    );
                                    let filename: Option<&str> = download
                                        .filename
                                        .split('\\')
                                        .next_back();
                                    match filename {
                                                        Some(filename) => {
                                                            match download_peer.download_file(
                                                                client_context_clone,
                                                                Some(download.clone()),
                                                                None
                                                            ) {
                                                                Ok((download, filename)) => {
                                                                    download.sender.send(DownloadStatus::Completed).unwrap();
                                                                    info!("Successfully downloaded {} bytes to {}", download.size, filename);
                                                                }
                                                                Err(e) => {
                                                                    download.sender.send(DownloadStatus::Failed).unwrap();
                                                                    error!(
                                                                        "Failed to download file '{}' from {}:{} (token: {}) - Error: {}", 
                                                                        filename, peer.host, peer.port, download.token, e
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        None => error!("Cant find filename to save download: {:?}", download.filename),
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
                        if client_context
                            .read()
                            .unwrap()
                            .peers
                            .contains_key(&new_peer.username)
                        {
                            debug!(
                                "Already connected to {}",
                                new_peer.username
                            );
                        } else if let Some(server_sender) =
                            &client_context.read().unwrap().server_sender
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

                        if let Some(_peer) =
                            client_context.read().unwrap().peers.get(&username)
                        {
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
                            Self::connect_to_peer(
                                peer,
                                client_context.clone(),
                                own_username.clone(),
                                None,
                            );
                        }
                    }
                    ClientOperation::UpdateDownloadTokens(
                        transfer,
                        username,
                    ) => {
                        let mut context = client_context.write().unwrap();

                        let key_to_remove = context
                            .download_tokens
                            .iter()
                            .find_map(|(key, d)| {
                                if d.username == username
                                    && d.filename == transfer.filename
                                {
                                    Some((*key, d.clone()))
                                } else {
                                    None
                                }
                            });

                        if let Some((key, download)) = key_to_remove {
                            trace!(
                                "[client] UpdateDownloadTokens found {key}, transfer: {:?}",
                                transfer
                            );

                            context.download_tokens.insert(
                                transfer.token,
                                Download {
                                    username: username.clone(),
                                    filename: transfer.filename,
                                    token: transfer.token,
                                    size: transfer.size,
                                    download_directory: download
                                        .download_directory,
                                    sender: download.sender.clone(),
                                },
                            );
                            context.download_tokens.remove(&key);
                        }
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
        let client_context2 = client_context.clone();
        let unlocked_context = client_context.read().unwrap();

        trace!("[client] connect_to_peer: {}", peer.username);
        if let Some(sender) = unlocked_context.sender.clone() {
            let peer_clone = peer.clone();
            let sender_clone = sender;
            unlocked_context.thread_pool.execute(move || {
                    trace!("[client] connecting to {}, with connection_type: {}, and token {:?}", peer.username, peer.connection_type, peer.token);
                    match peer.connection_type {
                        ConnectionType::P => {
                            let default_peer =
                                DefaultPeer::new(peer_clone, sender_clone);

                            let connect_result = match stream {
                                    Some(s) => default_peer.connect_with_socket(s, None),
                                    _ => default_peer.connect()
                                };

                            match connect_result {
                                Ok(p) => {
                                    trace!("[client] connected to: {}", peer.username);
                                    client_context2.write().unwrap().peers.insert(peer.username, p);
                                }
                                Err(e) => {
                                    trace!(
                                        "Can't connect to {:?} {:?}:{:?} {:?} - {:?}",
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
                                    trace!("[client] downloading from: {}, {:?}", peer.username, peer.token);
                                    let download_peer = DownloadPeer::new(
                                        peer.username,
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
                                        Ok((download, filename)) => {
                                            trace!("[client] downloaded {} bytes {:?} ", filename, download.size);
                                            download.sender.send(DownloadStatus::Completed).unwrap(); 
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
                });
        } else {
            debug!("[client] Peer already connected: {}", peer.username);
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
}
