use crate::{
    error::{Result, SoulseekRs},
    peer::{
        listen::Listen, ConnectionType, DefaultPeer, DownloadPeer, NewPeer,
        Peer,
    },
    server::{PeerAddress, Server, ServerOperation},
    types::{Download, FileSearchResult},
    utils::{md5, thread_pool::ThreadPool},
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
    GetPeerAddressResponse {
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    },
}
struct ClientContext {
    peers: HashMap<String, DefaultPeer>,
    sender: Option<Sender<ClientOperation>>,
    server_sender: Option<Sender<crate::server::ServerOperation>>,
    search_results: Vec<FileSearchResult>,
    downloads: HashMap<u32, Download>,
    thread_pool: ThreadPool,
}
impl ClientContext {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            sender: None,
            server_sender: None,
            search_results: Vec::new(),
            downloads: HashMap::new(),
            thread_pool: ThreadPool::new(MAX_THREADS),
        }
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
        let context = Arc::new(Mutex::new(ClientContext::new()));
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

        let client_sender = sender.clone();

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
        use crate::types::{DownloadResult, DownloadStatus};
        use std::time::{Duration, Instant};

        let start_time = Instant::now();

        let hash = md5::md5(&filename);
        let token = u32::from_str_radix(&hash[0..5], 16)?;

        let download = Download {
            username: username.clone(),
            filename: filename.clone(),
            token,
            size,
        };

        let mut context = self.context.lock().unwrap();
        context.downloads.insert(token, download.clone());
        let download_initiated = context
            .peers
            .get(&username)
            .map(|p| p.transfer_request(download.clone()))
            .is_some();

        drop(context);

        let timeout = Duration::from_secs(150);
        let check_interval = Duration::from_millis(100);

        if !download_initiated {
            return Ok(DownloadResult {
                filename,
                username,
                status: DownloadStatus::Failed,
                elapsed_time: start_time.elapsed(),
            });
        }

        while start_time.elapsed() < timeout {
            std::thread::sleep(check_interval);
        }

        Ok(DownloadResult {
            filename,
            username,
            status: DownloadStatus::TimedOut,
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
                        // {
                        //     let mut context = client_context.lock().unwrap();
                        //     if let Some(default_peer) =
                        //         context.peers.remove(&peer.username)
                        //     {
                        //         default_peer.disconnect();
                        //     }
                        // }

                        let maybe_download = {
                            let client_context = client_context.lock().unwrap();
                            client_context.downloads.get(&token).cloned()
                        };
                        let own_username = own_username.clone();

                        match maybe_download {
                            Some(download) => {
                                thread::spawn(move || {
                                    let download_peer = DownloadPeer::new(
                                        download.username.clone(),
                                        peer.host.clone(),
                                        peer.port,
                                        download.token,
                                        false,
                                        own_username,
                                    );
                                    let filename: Option<&str> =
                                        download.filename.split('\\').last();
                                    match filename {
                                        Some(filename) => {
                                            match download_peer.download_file(
                                                Some(download.size as usize),
                                                Some(format!("/tmp/{}", filename)),
                                            ) {
                                                Ok(bytes) => {
                                                    info!("Successfully downloaded {} bytes to /tmp/{}", bytes, filename);
                                                }
                                                Err(e) => {
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
                            trace!("[client] starting downloading from: {}", peer.username);
                            let context = client_context2.lock().unwrap();
                            trace!("[client] downloads: {:?}", context.downloads);
                            trace!("[client] peer.token: {:?}", peer.token);
                            let download =
                                context.downloads.get(&peer.token.unwrap());

                            match download {
                                Some(download) => {
                                    trace!("[client] downloading from: {}", peer.username);
                                    let download_peer = DownloadPeer::new(
                                        peer.username,
                                        peer.host,
                                        peer.port,
                                        peer.token.unwrap(),
                                        true,
                                        own_username.clone(),
                                    );

                                    let filename: Option<&str> =
                                        download.filename.split('\\').last();
                                    match filename {
                                        Some(filename) => {
                                            trace!("[client] startting downloading file: {}", filename);
                                            download_peer
                                                .download_file(
                                                    Some(
                                                        download.size as usize,
                                                    ),
                                                    Some(format!(
                                                            "/tmp/{}",
                                                            filename
                                                    )),
                                                )
                                                .unwrap();
                                            }
                                        None => todo!(),
                                    }
                                }
                                None => todo!(),
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
}
