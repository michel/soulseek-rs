use crate::{
    peer::{DefaultPeer, Peer},
    server::{PeerAddress, Server},
    types::{FileSearchResult, Transfer},
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
    search_results: Vec<FileSearchResult>,
    downloads: HashMap<i32, Transfer>,
    thread_pool: ThreadPool,
}
impl ClientContext {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            sender: None,
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
    pub fn new(address: PeerAddress, username: String, password: String) -> Self {
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
        let (sender, message_reader): (Sender<ClientOperation>, Receiver<ClientOperation>) =
            mpsc::channel();

        self.context.lock().unwrap().sender = Some(sender.clone());

        // self.read_form_channel(message_reader);
        self.server = match Server::new(self.address.clone(), sender) {
            Ok(server) => {
                info!(
                    "Connected to server at {}:{}",
                    server.get_address().get_host(),
                    server.get_address().get_port()
                );

                Self::listen_to_client_operations(message_reader, self.context.clone());
                Some(server)
            }
            Err(e) => {
                error!("Error connecting to server: {}", e);
                None
            }
        };
    }

    pub fn login(&self) -> Result<bool, std::io::Error> {
        // Attempt to login
        info!("Logging in as {}", self.username);
        if let Some(server) = &self.server {
            let result = server.login(&self.username, &self.password);
            if result.unwrap() {
                Ok(true)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Error logging in",
                ))
            }
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Not connected to server",
            ))
        }
    }

    #[allow(dead_code)]
    pub fn remove_peer(&self, username: &str) {
        let mut context = self.context.lock().unwrap();
        if let Some(peer) = context.peers.remove(username) {
            drop(peer); // Explicitly drop to trigger cleanup
        }
    }

    pub fn search(&self, query: &str, timeout: Duration) -> Vec<FileSearchResult> {
        info!("Searching for {}", query);
        if let Some(server) = &self.server {
            let hash = md5::md5(query);
            let token = i32::from_str_radix(&hash[0..5], 16).unwrap();
            server.file_search(token, query);
        } else {
            warn!("Not connected to server");
        }

        let start = Instant::now();
        loop {
            if start.elapsed() >= timeout {
                break;
            }
        }
        return self.context.lock().unwrap().search_results.clone();
    }

    pub fn download(&self, filename: String, username: String) {
        println!("Downloading {} from {}", filename, username);
        let context = self.context.lock().unwrap();
        context
            .peers
            .get(&username)
            .map(|p| p.transfer_request(filename));
    }

    fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<Mutex<ClientContext>>,
    ) {
        thread::spawn(move || loop {
            if let Ok(operation) = reader.recv() {
                match operation {
                    ClientOperation::ConnectToPeer(peer) => {
                        Self::connect_to_peer(peer, client_context.clone())
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
                    ClientOperation::TransferRequest(transfer) => {
                        client_context
                            .lock()
                            .unwrap()
                            .downloads
                            .insert(transfer.token, transfer);
                    }
                    ClientOperation::PierceFireWall(peer) => {
                        Self::pierce_firewall(peer, client_context.clone());
                    }
                }
            }
        });
    }

    fn connect_to_peer(peer: Peer, client_context: Arc<Mutex<ClientContext>>) {
        let context = client_context.clone();
        let unlocked_context = context.lock().unwrap();

        if let Some(sender) = &unlocked_context.sender {
            if !unlocked_context.peers.contains_key(&peer.username) {
                let peer_clone = peer.clone();
                let sender_clone = sender.clone();
                unlocked_context.thread_pool.execute(move || {
                    let default_peer = DefaultPeer::new(peer_clone, sender_clone);
                    match default_peer.connect() {
                        Ok(p) => {
                            let mut context = client_context.lock().unwrap();
                            context.peers.insert(peer.username, p);
                        }
                        Err(e) => {
                            // error!(
                            //     "Can't connect to {} {}:{} - {}",
                            //     peer.username, peer.host, peer.port, e
                            // );
                        }
                    }
                });
            }
        } else {
            error!("No sender found");
        }
    }
    fn pierce_firewall(peer: Peer, client_context: Arc<Mutex<ClientContext>>) {}
}
