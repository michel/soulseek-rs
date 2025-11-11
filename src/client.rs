use crate::actor::server_actor::{PeerAddress, ServerActor, ServerOperation};
use crate::actor::ActorHandle;
use crate::types::{DownloadResult, DownloadStatus};
use crate::{
    actor::{peer_registry::PeerRegistry, ActorSystem},
    error::{Result, SoulseekRs},
    peer::{ConnectionType, DownloadPeer, NewPeer, Peer},
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
const DEFALT_LISTEN_PORT: u32 = 2234;

#[derive(Debug)]

pub enum ClientOperation {
    NewPeer(NewPeer),
    ConnectToPeer(Peer),
    SearchResult(FileSearchResult),
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
    SetServerSender(Sender<ServerOperation>),
}
pub struct ClientContext {
    pub peer_registry: Option<PeerRegistry>,
    sender: Option<Sender<ClientOperation>>,
    server_sender: Option<Sender<ServerOperation>>,
    search_results: Vec<FileSearchResult>,
    pub download_tokens: HashMap<u32, Download>,
    actor_system: Arc<ActorSystem>,
}
impl Default for ClientContext {
    fn default() -> Self {
        Self::new()
    }
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
            search_results: Vec::new(),
            download_tokens: HashMap::new(),
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
    server_handle: Option<ActorHandle<ServerOperation>>,
    context: Arc<RwLock<ClientContext>>,
}

impl Client {
    pub fn new(
        address: PeerAddress,
        username: String,
        password: String,
        enable_listen: bool,
        listen_port: Option<u32>,
    ) -> Self {
        crate::utils::logger::init();
        Self {
            enable_listen,
            listen_port: listen_port.unwrap_or(DEFALT_LISTEN_PORT),
            address,
            username,
            password,
            context: Arc::new(RwLock::new(ClientContext::new())),
            server_handle: None,
        }
    }
    pub fn with_defaults(
        address: PeerAddress,
        username: String,
        password: String,
    ) -> Self {
        Self::new(address, username, password, false, None)
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
            let _ = handle.send(ServerOperation::Login {
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
        if let Some(ref registry) = context.peer_registry {
            if let Some(handle) = registry.remove_peer(username) {
                let _ = handle.stop();
            }
        }
    }

    pub fn search(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Vec<FileSearchResult>> {
        info!("Searching for {}", query);
        if let Some(handle) = &self.server_handle {
            let hash = md5::md5(query);
            let token = u32::from_str_radix(&hash[0..5], 16)?;
            let _ = handle.send(ServerOperation::FileSearch {
                token,
                query: query.to_string(),
            });
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
        info!("[client] Downloading {} from {}", filename, username);
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

    fn process_failed_uploads(
        client_context: Arc<RwLock<ClientContext>>,
        username: &str,
        filename: Option<&str>,
    ) {
        let context = client_context.read().unwrap();
        let download_tokens = &context.download_tokens;
        let failed_downloads: Vec<_> = download_tokens
            .iter()
            .filter(|(_, download)| {
                download.username == username
                    && (filename.is_none_or(|f| download.filename == *f))
            })
            .map(|(token, download)| {
                download.sender.send(DownloadStatus::Failed).unwrap();
                *token
            })
            .collect();

        failed_downloads.iter().for_each(|token| {
            client_context
                .write()
                .unwrap()
                .download_tokens
                .remove(token);
        });
    }

    fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || loop {
            match reader.recv() {
                Ok(operation) => {
                    match operation {
                        ClientOperation::ConnectToPeer(peer) => {
                            let client_context_clone = client_context.clone();
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
                        ClientOperation::SearchResult(file_search) => {
                            trace!("[client] SearchResult {:?}", file_search);
                            client_context
                                .write()
                                .unwrap()
                                .search_results
                                .push(file_search.clone());
                        }
                        ClientOperation::PeerDisconnected(username, error) => {
                            let context = client_context.read().unwrap();
                            if let Some(ref registry) = context.peer_registry {
                                if let Some(handle) =
                                    registry.remove_peer(&username)
                                {
                                    let _ = handle.stop();
                                }
                            }
                            if let Some(error) = error {
                                error!(
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
                                    .download_tokens
                                    .get(&token)
                                    .cloned()
                            };
                            let own_username = own_username.clone();
                            let client_context_clone = client_context.clone();

                            trace!(
                            "[client] DownloadFromPeer token: {} peer: {:?}",
                            token,
                            peer
                        );
                            match maybe_download {
                                Some(download) => {
                                    thread::spawn(move || {
                                        let download_peer = DownloadPeer::new(
                                            download.username.clone(),
                                            peer.host.clone(),
                                            peer.port,
                                            token,
                                            allowed,
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
                        ClientOperation::UploadFailed(username, filename) => {
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
        trace!("[client] connecting to {}, with connection_type: {}, and token {:?}", peer.username, peer.connection_type, peer.token);
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
                                username,
                                e
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
                    peer.username,
                    peer.token
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
                            filename,
                            download.size
                        );
                        download
                            .sender
                            .send(DownloadStatus::Completed)
                            .unwrap();
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
