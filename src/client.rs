use crate::{
    error::{Result, SoulseekRs},
    message::peer::distributed::SearchRequestInfo,
    message::server::MessageFactory,
    peer::{
        ConnectionType, DefaultPeer, DistributedPeer, DownloadPeer, Peer,
        PeerConnection,
    },
    server::{PeerAddress, Server, ServerOperation},
    share::{Shared, SharedFile},
    types::{Download, FileSearchResult},
    utils::{md5, thread_pool::ThreadPool},
    Transfer,
};
use std::{
    collections::HashMap,
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use crate::{debug, error, info, trace};
const MAX_THREADS: usize = 50;
pub enum ClientOperation {
    ConnectToPeer(Peer),
    SearchResult(FileSearchResult),
    PeerDisconnected(String),
    PierceFireWall(Peer),
    DownloadFromPeer(Vec<u8>, Peer),
    ChangeDownload(Transfer, String),
    RemoveDownload(Vec<u8>),
    DistributedSearch(SearchRequestInfo),
}
struct ClientContext {
    peers: HashMap<String, PeerConnection>,
    sender: Option<Sender<ClientOperation>>,
    server_sender: Option<Sender<crate::server::ServerOperation>>,
    search_results: Vec<FileSearchResult>,
    downloads: HashMap<Vec<u8>, Download>,
    thread_pool: ThreadPool,
    shared: Shared,
    peer_search_matches: HashMap<String, HashMap<Vec<u8>, Vec<SharedFile>>>,
    current_login: String,
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
            shared: Shared::new(),
            peer_search_matches: HashMap::new(),
            current_login: String::new(),
        }
    }
}
pub struct Client {
    address: PeerAddress,
    username: String,
    password: String,
    server: Option<Server>,
    context: Arc<Mutex<ClientContext>>,
    #[allow(dead_code)]
    shared_folders: Vec<String>,
}

impl Client {
    pub fn new(
        address: PeerAddress,
        username: String,
        password: String,
    ) -> Self {
        Self::new_with_shares(address, username, password, vec![])
    }

    pub fn new_with_shares(
        address: PeerAddress,
        username: String,
        password: String,
        shared_folders: Vec<String>,
    ) -> Self {
        crate::utils::logger::init();
        let context = Arc::new(Mutex::new(ClientContext::new()));
        debug!(
            "ThreadPool initialized with {} threads",
            context.lock().unwrap().thread_pool.thread_count()
        );

        // Scan shared folders
        for folder in &shared_folders {
            context.lock().unwrap().shared.scan_folder(folder);
        }

        Self {
            address,
            username,
            password,
            server: None,
            context,
            shared_folders,
        }
    }

    pub fn connect(&mut self) {
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        // Start the peer listener in a background thread
        let sender_for_listen = sender.clone();
        thread::spawn(move || {
            // Port 2234 matches what we send in SetWaitPort
            crate::peer::listen::Listen::start(2234, sender_for_listen);
        });

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
                // Store the current login username
                self.context.lock().unwrap().current_login =
                    self.username.clone();
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

        let token = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos() as u32;

        let token_bytes = token.to_le_bytes().to_vec();

        let download = Download {
            username: username.clone(),
            filename: filename.clone(),
            token: token_bytes.clone(),
            size,
        };

        let mut context = self.context.lock().unwrap();
        context.downloads.insert(token_bytes, download.clone());
        let download_initiated = context
            .peers
            .get(&username)
            .map(|p| p.transfer_request(download.clone()))
            .is_some();

        drop(context);

        let timeout = Duration::from_secs(60 * 5);
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
                    ClientOperation::DownloadFromPeer(token, peer) => {
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
                                        peer.host,
                                        peer.port,
                                        download.token,
                                        true,
                                        own_username,
                                    );
                                    download_peer
                                        .download_file(
                                            Some(download.size as usize),
                                            Some(String::from(
                                                "/tmp/download.txt",
                                            )),
                                        )
                                        .unwrap();
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
                    ClientOperation::ChangeDownload(transfer, username) => {
                        let mut ctx = client_context.lock().unwrap();
                        match ctx.downloads.get_mut(&transfer.token) {
                            Some(existing) => {
                                existing.filename = transfer.filename;
                                existing.username = username;
                                existing.size = transfer.size;
                                existing.token = transfer.token;
                            }
                            None => {
                                ctx.downloads.insert(
                                    transfer.token.clone(),
                                    Download {
                                        username,
                                        token: transfer.token.clone(),
                                        filename: transfer.filename,
                                        size: transfer.size,
                                    },
                                );
                            }
                        }
                    }
                    ClientOperation::RemoveDownload(token) => {
                        let mut ctx = client_context.lock().unwrap();
                        ctx.downloads.remove(&token).unwrap();
                    }
                    ClientOperation::DistributedSearch(search_info) => {
                        debug!(
                            "Received distributed search from {}: '{}'",
                            search_info.username, search_info.query
                        );

                        // Search our shared files
                        let mut ctx = client_context.lock().unwrap();
                        let local_matches =
                            ctx.shared.search(&search_info.query);

                        if !local_matches.is_empty() {
                            debug!(
                                "Found {} local matches for query '{}'",
                                local_matches.len(),
                                search_info.query
                            );

                            // Check if we're already connected to the searching user
                            if let Some(peer_conn) =
                                ctx.peers.get(&search_info.username)
                            {
                                // Send the results immediately
                                peer_conn.file_search_result(
                                    local_matches,
                                    search_info.ticket,
                                    ctx.current_login.clone(),
                                );
                            } else {
                                // Cache the results for when we connect
                                let user_matches = ctx
                                    .peer_search_matches
                                    .entry(search_info.username.clone())
                                    .or_insert_with(HashMap::new);
                                user_matches
                                    .insert(search_info.ticket, local_matches);

                                // Request peer address from server to establish connection
                                if let Some(server_sender) = &ctx.server_sender
                                {
                                    debug!("Caching search results for {} and requesting peer address", search_info.username);
                                    // Send GetPeerAddress message through the server
                                    server_sender.send(ServerOperation::SendMessage(
                                        MessageFactory::build_get_peer_address_message(&search_info.username)
                                    )).unwrap();
                                }
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

                                    // Check if we have cached search results for this peer
                                    if let Some(cached_results) = context.peer_search_matches.get(&peer.username) {
                                        for (ticket, files) in cached_results.clone() {
                                            p.file_search_result(files, ticket, context.current_login.clone());
                                        }
                                        // Clear the cache after sending
                                        context.peer_search_matches.remove(&peer.username);
                                    }
                                    context.peers.insert(peer.username, PeerConnection::Default(p));
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
                            debug!("Received ConnectToPeer with ConnectionType::F for {}", peer.username);
                            let context = client_context.lock().unwrap();
                            if let Some(token) = &peer.token {
                                if let Some(download) = context.downloads.get(token) {
                                    let download_clone = download.clone();
                                    let download_peer = DownloadPeer::new(
                                        peer.username.clone(),
                                        peer.host,
                                        peer.port,
                                        token.clone(),
                                        false,  // no_pierce = false for F-type
                                        own_username.clone(),
                                    );
                                    drop(context);  // Release the lock before spawning thread
                                    thread::spawn(move || {
                                        match download_peer.download_file(
                                            Some(download_clone.size as usize),
                                            Some(String::from("/tmp/download.txt")),
                                        ) {
                                            Ok(_) => debug!("Download completed successfully"),
                                            Err(e) => error!("Download failed: {}", e),
                                        }
                                    });
                                } else {
                                    error!("No download found for token {:?}", token);
                                }
                            } else {
                                error!("No token provided in F-type ConnectToPeer");
                            }
                        }
                        ConnectionType::D => {
                            let dist_peer = DistributedPeer::new(peer_clone, sender_clone);
                            match dist_peer.connect(&own_username) {
                                Ok(p) => {
                                    info!("Connected to DistributedPeer: {}", peer.username);
                                    let mut context = client_context.lock().unwrap();
                                    context.peers.insert(peer.username, PeerConnection::Distributed(p));
                                }
                                Err(e) => {
                                    error!("Failed to connect to DistributedPeer {}: {}", peer.username, e);
                                }
                            }
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
            if let Some(token) = peer.token.clone() {
                match server_sender
                    .send(ServerOperation::PierceFirewall(token.clone()))
                {
                    Ok(_) => debug!(
                        "Sent PierceFirewall message with token: {:?}",
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
