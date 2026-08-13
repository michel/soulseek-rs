use super::{
    Arc, Client, ClientContext, ClientOperation, ConnectionType, Download,
    DownloadPeer, DownloadStatus, Duration, Instant, Peer, PeerMessage,
    PeerRegistry, Receiver, RwLock, RwLockExt, ServerMessage,
    build_search_response, debug, error, info, mpsc, next_connect_token,
    thread, trace, warn,
};
use crate::message::server::MessageFactory;

const CONNECT_SWEEP_INTERVAL: Duration = Duration::from_secs(1);

impl Client {
    pub(crate) fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || {
            let mut last_sweep = Instant::now();
            loop {
                let next = reader.recv_timeout(CONNECT_SWEEP_INTERVAL);
                if last_sweep.elapsed() >= CONNECT_SWEEP_INTERVAL {
                    last_sweep = Instant::now();
                    Self::sweep_expired_connects(&client_context);
                }
                let operation = match next {
                    Ok(operation) => operation,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                match operation {
                    ClientOperation::ConnectToPeer(peer) => {
                        if matches!(peer.connection_type, ConnectionType::F) {
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
                        } else {
                            Self::connect_to_peer(
                                peer,
                                client_context.clone(),
                                own_username.clone(),
                                None,
                            );
                        }
                    }
                    ClientOperation::SearchResult(search_result) => {
                        trace!("[client] SearchResult {:?}", search_result);
                        let mut context = match client_context.write_safe() {
                            Ok(c) => c,
                            Err(e) => {
                                error!("[client] SearchResult write: {}", e);
                                continue;
                            }
                        };
                        let result_token = search_result.token;

                        // Find the search with matching token
                        for search in context.searches.values_mut() {
                            if search.token == result_token {
                                search.accept(search_result);
                                break;
                            }
                        }
                    }
                    ClientOperation::PeerDisconnected(id, username, error) => {
                        // Scope the read guard: process_failed_uploads
                        // below acquires a write lock on the same
                        // RwLock, which would self-deadlock the entire
                        // client ops loop if this read guard were still
                        // held on this thread. Evict only if this exact
                        // actor still occupies the slot, so a replaced
                        // actor's shutdown can't remove its successor.
                        {
                            let context = match client_context.read_safe() {
                                Ok(c) => c,
                                Err(e) => {
                                    error!(
                                        "[client] PeerDisconnected read: {}",
                                        e
                                    );
                                    continue;
                                }
                            };
                            if let Some(ref registry) = context.peer_registry
                                && let Some(handle) =
                                    registry.remove_peer_if(&username, id)
                            {
                                let _ = handle.stop();
                            }
                        }
                        // Only an error is evidence the peer is gone. A clean
                        // close — our idle reaper, or a remote client tidying
                        // an idle socket while it waits in our queue — must
                        // not throw away everything that peer has queued: the
                        // connection comes back, the queue cannot. A departed
                        // peer wedging uploads shut is still covered: its
                        // erroring transfer fails within the socket deadlines
                        // and the error branch here frees its slots.
                        if let Some(error) = error {
                            warn!(
                                "[client] Peer {} disconnected with error: {:?}",
                                username, error
                            );
                            Self::process_failed_uploads(
                                client_context.clone(),
                                &username,
                                None,
                            );
                            Self::release_upload_slots(
                                &client_context,
                                &username,
                            );
                        }
                    }
                    ClientOperation::DownloadFromPeer(token, peer, allowed) => {
                        let maybe_download = match client_context.read_safe() {
                            Ok(ctx) => {
                                ctx.get_download_by_token(token).cloned()
                            }
                            Err(e) => {
                                error!("[client] DownloadFromPeer read: {}", e);
                                continue;
                            }
                        };
                        let own_username = own_username.clone();
                        let client_context_clone = client_context.clone();

                        trace!(
                            "[client] DownloadFromPeer token: {} peer: {:?}",
                            token, peer
                        );
                        let Some(download) = maybe_download else {
                            error!(
                                "Can't find download with token {:?}",
                                token
                            );
                            continue;
                        };
                        if matches!(
                            download.status,
                            DownloadStatus::InProgress { .. }
                                | DownloadStatus::Completed
                        ) {
                            debug!(
                                "[client] transfer token {} already claimed; \
                                 ignoring replayed TransferResponse",
                                token
                            );
                            continue;
                        }
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.update_download_with_status(
                                token,
                                DownloadStatus::InProgress {
                                    bytes_downloaded: 0,
                                    total_bytes: download.size,
                                    speed_bytes_per_sec: 0.0,
                                },
                            );
                        }

                        thread::spawn(move || {
                            let download_peer = DownloadPeer::new(
                                download.username.clone(),
                                peer.host.clone(),
                                peer.port,
                                token,
                                allowed,
                                own_username,
                            );
                            let Some(filename) =
                                download.filename.split('\\').next_back()
                            else {
                                error!(
                                    "Cant find filename to save download: {:?}",
                                    download.filename
                                );
                                return;
                            };
                            match download_peer.download_file(
                                client_context_clone.clone(),
                                Some(download.clone()),
                                None,
                            ) {
                                Ok((download, filename)) => {
                                    let _ = download
                                        .sender
                                        .send(DownloadStatus::Completed);
                                    match client_context_clone.write_safe() {
                                        Ok(mut ctx) => ctx
                                            .update_download_with_status(
                                                download.token,
                                                DownloadStatus::Completed,
                                            ),
                                        Err(e) => error!(
                                            "[client] download complete write: {}",
                                            e
                                        ),
                                    }
                                    info!(
                                        "Successfully downloaded {} bytes to {}",
                                        download.size, filename
                                    );
                                }
                                Err(e) => {
                                    let reason = Some(e.to_string());
                                    let _ = download.sender.send(
                                        DownloadStatus::Failed(reason.clone()),
                                    );
                                    match client_context_clone.write_safe() {
                                        Ok(mut ctx) => ctx
                                            .update_download_with_status(
                                                download.token,
                                                DownloadStatus::Failed(reason),
                                            ),
                                        Err(e) => error!(
                                            "[client] download failed write: {}",
                                            e
                                        ),
                                    }
                                    error!(
                                        "Failed to download file '{}' from {}:{} (token: {}) - Error: {}",
                                        filename,
                                        peer.host,
                                        peer.port,
                                        download.token,
                                        e
                                    );
                                }
                            }
                        });
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
                            username,
                            host,
                            port,
                            obfuscation_type,
                            obfuscated_port
                        );

                        // Port 0 is the server saying it does not know
                        // where this user listens (no SetWaitPort yet,
                        // or firewalled). It is not an address: caching
                        // it poisons every later lookup, and an upload
                        // dialled at it dies with "Can't assign
                        // requested address" and is dropped on the
                        // floor. Leave those uploads queued for a later
                        // resolution instead.
                        //
                        // The connect attempt further down still runs:
                        // its failure is exactly what makes an
                        // unreachable peer fall back to the
                        // server-brokered path.
                        let waiting_serves = if port == 0 {
                            warn!(
                                "[client] server reports no listening port for {}",
                                username
                            );
                            Vec::new()
                        } else {
                            match client_context.write_safe() {
                                Ok(mut ctx) => {
                                    ctx.cache_peer_address(
                                        &username,
                                        host.clone(),
                                        port,
                                    );
                                    ctx.pending_serves
                                        .remove(&username)
                                        .unwrap_or_default()
                                }
                                Err(_) => Vec::new(),
                            }
                        };
                        for token in waiting_serves {
                            Self::spawn_serve(
                                &client_context,
                                &own_username,
                                token,
                                host.clone(),
                                port,
                            );
                        }

                        let peer_exists = match client_context.read_safe() {
                            Ok(ctx) => ctx
                                .peer_registry
                                .as_ref()
                                .is_some_and(|r| r.contains(&username)),
                            Err(e) => {
                                error!(
                                    "[client] GetPeerAddressResponse read: {}",
                                    e
                                );
                                continue;
                            }
                        };

                        // Existing peer: skip re-registration. Reconnect
                        // policy on conflict is intentionally undecided.
                        if !peer_exists {
                            let peer = Peer::new(
                                username,
                                ConnectionType::P,
                                host,
                                port,
                                None,
                                0,
                                // obfuscation_type is a small enum; a
                                // real obfuscated_port is a full u16 and
                                // must not be truncated into a u8 (which
                                // panicked and took down the ops thread).
                                u8::try_from(obfuscation_type).unwrap_or(0),
                                obfuscated_port,
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
                        let mut context = match client_context.write_safe() {
                            Ok(c) => c,
                            Err(e) => {
                                error!(
                                    "[client] UpdateDownloadTokens write: {}",
                                    e
                                );
                                continue;
                            }
                        };

                        let download_to_update =
                            context.get_downloads().iter().find_map(|d| {
                                if d.username == username
                                    && d.filename == transfer.filename
                                {
                                    Some((d.token, d.clone()))
                                } else {
                                    None
                                }
                            });

                        if let Some((old_token, download)) = download_to_update
                        {
                            trace!(
                                "[client] UpdateDownloadTokens found {old_token}, transfer: {:?}",
                                transfer
                            );

                            context.add_download(Download {
                                token: transfer.token,
                                size: transfer.size,
                                ..download
                            });
                            context.remove_download(old_token);
                        }

                        // Only now invite the file connection: it is
                        // matched by this token, which is recorded as
                        // of the line above. Answering any earlier
                        // races the peer's connection against our own
                        // bookkeeping.
                        let registry = context.peer_registry.clone();
                        drop(context);
                        if let Some(registry) = registry {
                            let response =
                                MessageFactory::build_transfer_response_message(
                                    transfer,
                                );
                            let _ = registry.send_to_peer(
                                &username,
                                PeerMessage::SendMessage(response),
                            );
                        }
                    }
                    ClientOperation::UploadFailed(username, filename) => {
                        Self::process_failed_uploads(
                            client_context.clone(),
                            &username,
                            Some(&filename),
                        );
                    }
                    ClientOperation::PlaceInQueueUpdate {
                        username,
                        filename,
                        place,
                    } => match client_context.write_safe() {
                        Ok(mut ctx) => {
                            let updated = ctx.downloads.update_queue_position(
                                &username, &filename, place,
                            );
                            if !updated {
                                debug!(
                                    "[client] PlaceInQueueUpdate: no matching download for {}/{}",
                                    username, filename
                                );
                            }
                        }
                        Err(e) => {
                            error!("[client] PlaceInQueueUpdate write: {}", e);
                        }
                    },
                    ClientOperation::SetServerSender(sender) => {
                        match client_context.write_safe() {
                            Ok(mut ctx) => {
                                ctx.server_sender = Some(sender);
                                debug!("[client] Server sender initialized");
                            }
                            Err(e) => {
                                error!("[client] SetServerSender write: {}", e);
                            }
                        }
                    }
                    ClientOperation::PrivateMessageReceived(user_message) => {
                        match client_context.write_safe() {
                            Ok(mut ctx) => {
                                ctx.push_private_message(user_message);
                            }
                            Err(e) => error!(
                                "[client] PrivateMessageReceived write: {}",
                                e
                            ),
                        }
                    }
                    ClientOperation::UserStatusReceived {
                        username,
                        status,
                        privileged,
                    } => match client_context.write_safe() {
                        Ok(mut ctx) => {
                            ctx.apply_user_status(username, status, privileged);
                        }
                        Err(e) => {
                            error!("[client] UserStatusReceived write: {}", e);
                        }
                    },
                    ClientOperation::UserStatsReceived {
                        username,
                        average_speed,
                        shared_files,
                        shared_folders,
                    } => match client_context.write_safe() {
                        Ok(mut ctx) => ctx.apply_user_stats(
                            username,
                            average_speed,
                            shared_files,
                            shared_folders,
                        ),
                        Err(e) => {
                            error!("[client] UserStatsReceived write: {}", e);
                        }
                    },
                    ClientOperation::RoomEvent(event) => {
                        match client_context.write_safe() {
                            Ok(mut ctx) => ctx.apply_room_event(event),
                            Err(e) => error!("[client] RoomEvent write: {}", e),
                        }
                    }
                    ClientOperation::WishlistInterval(seconds) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.wishlist_interval = Some(seconds);
                        }
                    }
                    ClientOperation::PeerConnected(username) => {
                        // A control connection just came up — one we dialled,
                        // or an inbound one the listener registered. Flush any
                        // downloads that were queued for this peer while it
                        // was unreachable. Collect under a read guard, then
                        // act without it held.
                        let (registry, files): (
                            Option<PeerRegistry>,
                            Vec<String>,
                        ) = match client_context.read_safe() {
                            Ok(ctx) => (
                                ctx.peer_registry.clone(),
                                ctx.get_downloads()
                                    .iter()
                                    .filter(|d| {
                                        d.username == username
                                            && matches!(
                                                d.status,
                                                DownloadStatus::Queued
                                            )
                                    })
                                    .map(|d| d.filename.clone())
                                    .collect(),
                            ),
                            Err(e) => {
                                error!("[client] PeerConnected read: {}", e);
                                continue;
                            }
                        };
                        // Also flush any peer messages (e.g. search
                        // responses) queued while connecting.
                        let queued_messages = client_context
                            .write_safe()
                            .map(|mut ctx| ctx.take_peer_messages(&username))
                            .unwrap_or_default();
                        if let Some(registry) = registry {
                            for filename in files {
                                let _ =
                                    registry.queue_upload(&username, filename);
                            }
                            for message in queued_messages {
                                let _ = registry.send_to_peer(
                                    &username,
                                    PeerMessage::SendMessage(message),
                                );
                            }
                        }
                    }
                    ClientOperation::IncomingSearch {
                        username,
                        token,
                        query,
                    } => {
                        // Don't answer our own distributed search.
                        if username == own_username {
                            continue;
                        }
                        let response = match client_context.read_safe() {
                            Ok(ctx) => build_search_response(
                                &ctx.shares,
                                &own_username,
                                token,
                                &query,
                            ),
                            Err(e) => {
                                error!("[client] IncomingSearch read: {}", e);
                                continue;
                            }
                        };
                        let Some(message) = response else {
                            continue; // no matching shares
                        };

                        // Deliver to the searcher: send now if we have a
                        // control connection, else open one and queue.
                        let (connected, registry, server_sender) =
                            match client_context.read_safe() {
                                Ok(ctx) => (
                                    ctx.peer_registry
                                        .as_ref()
                                        .is_some_and(|r| r.contains(&username)),
                                    ctx.peer_registry.clone(),
                                    ctx.server_sender.clone(),
                                ),
                                Err(_) => continue,
                            };
                        if connected {
                            if let Some(registry) = registry {
                                let _ = registry.send_to_peer(
                                    &username,
                                    PeerMessage::SendMessage(message),
                                );
                            }
                        } else {
                            if let Ok(mut ctx) = client_context.write_safe() {
                                ctx.queue_peer_message(&username, message);
                            }
                            if let Some(sender) = server_sender {
                                let _ = sender.send(
                                    ServerMessage::GetPeerAddress(username),
                                );
                            }
                        }
                    }
                    ClientOperation::QueueUpload {
                        requester_key,
                        filename,
                    } => {
                        // The peer served next may not be this one.
                        match client_context.write_safe() {
                            Ok(mut ctx) => {
                                let Some(file) = ctx.shares.get(&filename)
                                else {
                                    debug!(
                                        "[client] QueueUpload for unknown file {}",
                                        filename
                                    );
                                    continue;
                                };
                                let size = file.size;
                                let real_path = file.real_path.clone();
                                ctx.enqueue_upload(
                                    &requester_key,
                                    &filename,
                                    real_path,
                                    size,
                                );
                            }
                            Err(e) => {
                                error!("[client] QueueUpload write: {}", e);
                                continue;
                            }
                        }
                        Self::pump_upload_queue(&client_context);
                    }
                    ClientOperation::PlaceInQueueRequested {
                        requester_key,
                        filename,
                    } => {
                        let (registry, place) = match client_context.read_safe()
                        {
                            Ok(ctx) => (
                                ctx.peer_registry.clone(),
                                ctx.place_in_queue(&requester_key, &filename),
                            ),
                            Err(_) => continue,
                        };
                        // Silence would leave the peer guessing; a file
                        // no longer queued is being served, which is
                        // place 0 by the same convention Nicotine+ uses.
                        if let Some(registry) = registry {
                            let _ = registry.send_to_peer(
                                &requester_key,
                                PeerMessage::SendMessage(
                                    MessageFactory::build_place_in_queue_response(
                                        &filename,
                                        place.unwrap_or(0),
                                    ),
                                ),
                            );
                        }
                    }
                    ClientOperation::PrivilegedUsers(users) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            debug!(
                                "[client] {} privileged users listed",
                                users.len()
                            );
                            ctx.set_privileged_users(users);
                        }
                        // Someone already waiting may have just been
                        // outranked, but a slot that is free should
                        // still be filled.
                        Self::pump_upload_queue(&client_context);
                    }
                    ClientOperation::OwnPrivileges(seconds) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.own_privileges = Some(seconds);
                        }
                    }
                    ClientOperation::StartUpload { token } => {
                        // The peer accepted our offer: resolve their
                        // address (from the code-9 GetPeerAddress) and
                        // stream the file, or queue until it resolves.
                        let (job_addr, downloader) = match client_context
                            .read_safe()
                        {
                            Ok(ctx) => {
                                let Some(job) = ctx.uploads.get(&token) else {
                                    continue;
                                };
                                (
                                    ctx.peer_address(&job.downloader),
                                    job.downloader.clone(),
                                )
                            }
                            Err(_) => continue,
                        };
                        if let Some((host, port)) = job_addr {
                            Self::spawn_serve(
                                &client_context,
                                &own_username,
                                token,
                                host,
                                port,
                            );
                        } else {
                            if let Ok(mut ctx) = client_context.write_safe() {
                                ctx.pending_serves
                                    .entry(downloader.clone())
                                    .or_default()
                                    .push(token);
                            }
                            if let Ok(ctx) = client_context.read_safe()
                                && let Some(sender) = ctx.server_sender.clone()
                            {
                                let _ = sender.send(
                                    ServerMessage::GetPeerAddress(downloader),
                                );
                            }
                        }
                    }
                    ClientOperation::ShareListRequested { requester_key } => {
                        // Reply with our full shared-file listing.
                        let (registry, message) = match client_context
                            .read_safe()
                        {
                            Ok(ctx) => {
                                let dirs = ctx
                                    .shares
                                    .directories()
                                    .into_iter()
                                    .map(|(name, files)| {
                                        crate::message::peer::SharedDirectory {
                                            name,
                                            files,
                                        }
                                    })
                                    .collect::<Vec<_>>();
                                (
                                    ctx.peer_registry.clone(),
                                    crate::message::peer::build_shared_file_list(&dirs),
                                )
                            }
                            Err(_) => continue,
                        };
                        if let Some(registry) = registry {
                            let _ = registry.send_to_peer(
                                &requester_key,
                                PeerMessage::SendMessage(message),
                            );
                        }
                    }
                    ClientOperation::BrowseResult {
                        username,
                        directories,
                    } => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.store_browse_result(username, directories);
                        }
                    }
                    ClientOperation::PeerConnectFailed(id, username) => {
                        // Direct connect failed: ask the server to
                        // broker it. Register a correlation token, then
                        // send ConnectToPeer so the (firewalled) peer
                        // connects back to our listener quoting it.
                        let token = next_connect_token();
                        let server_sender = match client_context.write_safe() {
                            Ok(mut ctx) => {
                                // Reap the dead outbound actor so it
                                // stops pinning a pool worker and no
                                // longer shadows the brokered reconnect
                                // (a stale registry entry would make
                                // later downloads queue into a dead,
                                // streamless actor and hang). Identity-
                                // aware so a newer namesake is untouched.
                                if let Some(handle) =
                                    ctx.peer_registry.as_ref().and_then(|r| {
                                        r.remove_peer_if(&username, id)
                                    })
                                {
                                    let _ = handle.stop();
                                }
                                ctx.add_pending_connect(
                                    token,
                                    username.clone(),
                                );
                                ctx.server_sender.clone()
                            }
                            Err(e) => {
                                error!(
                                    "[client] PeerConnectFailed write: {}",
                                    e
                                );
                                continue;
                            }
                        };
                        let Some(sender) = server_sender else {
                            continue;
                        };
                        let msg = crate::message::server::MessageFactory::build_connect_to_peer(
                            token,
                            &username,
                            ConnectionType::P,
                        );
                        let _ = sender.send(ServerMessage::SendMessage(msg));
                    }
                }
            }
        });
    }

    fn sweep_expired_connects(client_context: &Arc<RwLock<ClientContext>>) {
        let expired = match client_context.write_safe() {
            Ok(mut ctx) => ctx.take_expired_connects(Instant::now()),
            Err(_) => return,
        };
        for username in expired {
            Self::fail_queued_downloads(client_context, &username);
        }
    }
}
