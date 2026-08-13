use super::{
    Arc, Client, ClientContext, ClientOperation, ConnectionType, DownloadPeer,
    DownloadStatus, Listen, Peer, PeerRegistry, Receiver, Result, RwLock,
    RwLockExt, Sender, ServerActor, ServerMessage, Shares, SoulseekRs,
    TcpStream, error, info, mpsc, thread, trace, warn,
};

/// Ceiling on the wait for a login verdict. Generous enough for a slow server
/// and a retrying connect, short enough that an unattended caller ends.
const LOGIN_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(45);

/// Soft open-file limit below which a session should expect "too many open
/// files": every peer holds a socket, and one busy search talks to hundreds
/// of them.
const COMFORTABLE_OPEN_FILES: u64 = 1024;

/// Say up front when the process cannot hold a busy session's sockets, so an
/// operator reads a warning with a number in it instead of an EMFILE storm an
/// hour later. The embedding process owns its rlimits; the library only looks.
#[cfg(unix)]
fn warn_if_open_file_limit_is_low() {
    // SAFETY: getrlimit only fills the struct it is handed.
    let limit = unsafe {
        let mut limit = std::mem::zeroed::<libc::rlimit>();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) != 0 {
            return;
        }
        limit.rlim_cur
    };
    if limit < COMFORTABLE_OPEN_FILES {
        warn!(
            "only {limit} open files allowed; a busy session wants \
             {COMFORTABLE_OPEN_FILES} or more — expect \"too many open \
             files\" under load"
        );
    }
}

#[cfg(not(unix))]
fn warn_if_open_file_limit_is_low() {}

impl Client {
    pub fn connect(&mut self) -> Result<()> {
        warn_if_open_file_limit_is_low();

        let (sender, message_reader): (
            Sender<ClientOperation>,
            Receiver<ClientOperation>,
        ) = mpsc::channel();

        let mut ctx = self.context.write_safe()?;
        let peer_registry = PeerRegistry::with_capacity(
            ctx.actor_system.clone(),
            sender.clone(),
            self.username.clone(),
            ctx.max_peers.clone(),
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
        ctx.shared_directories.clone_from(&self.shared_directories);

        // Bind before logging in: the port we advertise has to be the port we
        // hold, and a bind that fails outright is the caller's to see rather
        // than a panic on a thread nobody is watching.
        let listener = if self.enable_listen {
            let listener = Listen::bind(self.listen_port)?;
            self.bound_port = Some(listener.local_addr()?.port());
            Some(listener)
        } else {
            None
        };

        let mut server_actor = ServerActor::new(
            self.address.clone(),
            sender,
            self.bound_port.unwrap_or(self.listen_port),
            self.enable_listen,
            shared_folder_count,
            shared_file_count,
        );
        server_actor.set_session_watch(self.session.clone());

        self.server_handle = Some(ctx.actor_system.spawn_with_handle(
            server_actor,
            |actor, handle| {
                actor.set_self_handle(handle);
            },
        ));

        if let Some(listener) = listener {
            let client_sender = listen_sender;
            let context = self.context.clone();
            let own_username = self.username.clone();

            thread::spawn(move || {
                Listen::serve(&listener, client_sender, context, own_username);
            });
        }

        Self::listen_to_client_operations(
            message_reader,
            self.context.clone(),
            self.username.clone(),
        );

        Ok(())
    }

    /// Log in and wait for the server's verdict.
    ///
    /// The wait is bounded: the actor only answers once it has a working
    /// connection, so a server that never completes the TCP handshake would
    /// otherwise block the caller forever. A caller that hits
    /// [`SoulseekRs::Timeout`] can retry or give up, but it always regains
    /// control.
    pub fn login(&self) -> Result<bool> {
        info!("Logging in as {}", self.username);
        if let Some(handle) = &self.server_handle {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = handle.send(ServerMessage::Login {
                username: self.username.clone(),
                password: self.password.clone(),
                version: self.version,
                response: tx,
            });

            match rx.recv_timeout(LOGIN_RESPONSE_TIMEOUT) {
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

    pub fn set_max_peers(&self, max_peers: usize) {
        if let Ok(ctx) = self.context.read_safe() {
            ctx.max_peers
                .store(max_peers.max(1), std::sync::atomic::Ordering::Relaxed);
        }
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
        let peer_clone = peer.clone();
        trace!(
            "[client] connecting to {}, with connection_type: {}, and token {:?}",
            peer.username, peer.connection_type, peer.token
        );
        match peer.connection_type {
            ConnectionType::P => {
                let username = peer.username;

                let refused = {
                    let context = match client_context.read_safe() {
                        Ok(c) => c,
                        Err(e) => {
                            error!("[client] connect_to_peer read: {}", e);
                            return;
                        }
                    };
                    if let Some(ref registry) = context.peer_registry {
                        let protected = context.protected_peers();
                        match registry.register_peer_protected(
                            peer_clone, stream, None, &protected,
                        ) {
                            Ok(_) => false,
                            Err(e) => {
                                warn!(
                                    "[client] refusing peer connection for \
                                     {:?}: {:?}",
                                    username, e
                                );
                                true
                            }
                        }
                    } else {
                        trace!("PeerRegistry not initialized");
                        false
                    }
                };
                if refused {
                    Self::fail_queued_downloads(&client_context, &username);
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
}
