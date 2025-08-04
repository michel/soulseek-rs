use crate::{
    error::{Result, SoulseekRs},
    peer::{ConnectionType, DefaultPeer, DownloadPeer, Peer},
    server::{PeerAddress, Server, ServerOperation},
    types::{Download, FileSearchResult, Transfer},
    utils::{md5, thread_pool::ThreadPool},
};
use std::{
    collections::HashMap,
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    thread,
};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace};
const MAX_THREADS: usize = 100;
pub enum ClientOperation {
    ConnectToPeer(Peer),
    SearchResult(FileSearchResult),
    PeerDisconnected(String),
    TransferRequest(Transfer),
    PierceFireWall(Peer),
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
        Self {
            address,
            username,
            password,
            server: None,
            context: Arc::new(Mutex::new(ClientContext::new())),
        }
    }

    pub fn connect(&mut self) {
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        self.context.lock().unwrap().sender = Some(sender.clone());

        // self.read_form_channel(message_reader);
        self.server = match Server::new(self.address.clone(), sender) {
            Ok(server) => {
                info!(
                    "Connected to server at {}:{}",
                    server.get_address().get_host(),
                    server.get_address().get_port()
                );

                // Store the server sender in the client context
                self.context.lock().unwrap().server_sender =
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

        let timeout = Duration::from_secs(50);
        let check_interval = Duration::from_millis(100);

        if !download_initiated {
            return Ok(DownloadResult {
                filename,
                username,
                status: DownloadStatus::Failed,
                elapsed_time: start_time.elapsed(),
            });
        }

        // Non-blocking wait loop
        while start_time.elapsed() < timeout {
            // Check download status (for now just wait, actual download logic will be implemented later)
            std::thread::sleep(check_interval);

            // TODO: Check actual download progress here
            // For now, we'll just wait for the timeout
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
                        match peer.connection_type {
                            ConnectionType::P => (),
                            ConnectionType::F => {
                                debug!("Peer with F {:?}", peer)
                            }
                            ConnectionType::D => (),
                        };

                        Self::connect_to_peer(
                            peer,
                            client_context.clone(),
                            own_username.clone(),
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
                    ClientOperation::TransferRequest(transfer) => todo!(),
                }
            }
        });
    }

    fn connect_to_peer(
        peer: Peer,
        client_context: Arc<Mutex<ClientContext>>,
        own_username: String,
    ) {
        let context = client_context.clone();
        let unlocked_context = context.lock().unwrap();

        if let Some(sender) = &unlocked_context.sender {
            if !unlocked_context.peers.contains_key(&peer.username) {
                let peer_clone = peer.clone();
                let sender_clone = sender.clone();
                unlocked_context.thread_pool.execute(move || {
                    match peer.connection_type {
                        ConnectionType::P => {
                            let default_peer =
                                DefaultPeer::new(peer_clone, sender_clone);
                            match default_peer.connect() {
                                Ok(p) => {
                                    let mut context =
                                        client_context.lock().unwrap();
                                    context.peers.insert(peer.username, p);
                                }
                                Err(e) => {
                                    trace!(
                                        "Can't connect to {} {}:{} {:?} - {}",
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
                            let context = client_context.lock().unwrap();
                            // let download = if(Some(token)  {
                            //     let download = context.downloads.get(&peer.token.unwrap());
                            // }

                            let download_peer = DownloadPeer::new(
                                peer.username,
                                peer.host,
                                peer.port,
                                peer.token.unwrap(),
                                false,
                                own_username.clone(),
                            );
                            // download_peer.download_file(
                            //     );
                        }
                        ConnectionType::D => {
                            error!("ConnectionType::D not implemented")
                        }
                    }
                });
            }
        } else {
            error!("No sender found");
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
                    Ok(_) => debug!(
                        "Sent PierceFirewall message with token: {}",
                        token
                    ),
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
        // Also try to connect to the peer directly
        Self::connect_to_peer(peer, client_context, own_username);
    }
}
