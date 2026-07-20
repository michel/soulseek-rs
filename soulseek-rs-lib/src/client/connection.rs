use super::{
    Arc, Client, ClientContext, ClientOperation, ConnectionType, DownloadPeer,
    DownloadStatus, Listen, Peer, PeerRegistry, Receiver, Result, RwLock,
    RwLockExt, Sender, ServerActor, ServerMessage, Shares, SoulseekRs,
    TcpStream, debug, error, info, mpsc, thread, trace, warn,
};

impl Client {
    pub fn connect(&mut self) -> Result<()> {
        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        let mut ctx = self.context.write_safe()?;
        ctx.sender = Some(sender.clone());
        let peer_registry = PeerRegistry::new(
            ctx.actor_system.clone(),
            sender.clone(),
            self.username.clone(),
        );
        ctx.peer_registry = Some(peer_registry);

        let listen_sender = sender.clone();

        // Scan the shared directories once into the read-only index, and
        // report the real folder/file counts to the server on login.
        let roots: Vec<std::path::PathBuf> = self
            .shared_directories
            .iter()
            .filter(|dir| !dir.trim().is_empty())
            .map(std::path::PathBuf::from)
            .collect();
        let shares = if roots.is_empty() {
            Arc::new(Shares::empty())
        } else {
            let scanned = Shares::scan_many(&roots);
            info!(
                "Sharing {} files in {} folders from {} directories",
                scanned.file_count(),
                scanned.folder_count(),
                roots.len()
            );
            Arc::new(scanned)
        };
        let shared_folder_count = shares.folder_count();
        let shared_file_count = shares.file_count();
        ctx.shares = shares;

        let server_actor = ServerActor::new(
            self.address.clone(),
            sender,
            self.listen_port,
            self.enable_listen,
            shared_folder_count,
            shared_file_count,
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

    /// Ask the server for a peer's address and open a direct control
    /// connection to it. Downloads queued for that peer are sent automatically
    /// once the connection is established.
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
            .map_err(|_| SoulseekRs::NotConnected)?;
        Ok(())
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

    pub(crate) fn connect_to_peer(
        peer: Peer,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
        stream: Option<TcpStream>,
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
                let Some(token) = peer.token else {
                    error!(
                        "[client] F connection from {} without token",
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
                        match client_context.write_safe() {
                            Ok(mut ctx) => ctx.update_download_with_status(
                                download.token,
                                DownloadStatus::Completed,
                            ),
                            Err(e) => error!(
                                "[client] connect_to_peer F write: {}",
                                e
                            ),
                        }
                    }
                    Err(e) => {
                        trace!("[client] failed to download: {}", e);
                    }
                }
            }
            ConnectionType::D => {
                error!("ConnectionType::D not implemented");
            }
        }
    }

    pub(crate) fn pierce_firewall(
        peer: Peer,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        debug!("Piercing firewall for peer: {:?}", peer);

        let context = match client_context.read_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] pierce_firewall read: {}", e);
                return;
            }
        };
        if let Some(server_sender) = &context.server_sender {
            if let Some(token) = peer.token {
                match server_sender.send(ServerMessage::PierceFirewall(token)) {
                    Ok(()) => (),
                    Err(e) => {
                        error!("Failed to send PierceFirewall message: {}", e);
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
