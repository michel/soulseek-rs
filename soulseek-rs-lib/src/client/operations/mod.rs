use super::{
    Arc, Client, ClientContext, ClientOperation, ConnectionType, Download,
    DownloadPeer, DownloadStatus, Duration, Instant, Peer, PeerMessage,
    PeerRegistry, Receiver, RwLock, RwLockExt, ServerMessage,
    build_search_response, debug, error, info, mpsc, next_connect_token,
    thread, trace, warn,
};
use crate::message::server::MessageFactory;
use crate::peer::DownloadError;

mod peers;
mod search;
mod transfers;
mod tree;

const CONNECT_SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// How often to ask the peers holding our queued downloads where they sit
/// (peer code 51). Five minutes is what Nicotine+ uses: often enough that a
/// queue position on screen means something, rare enough that a peer with a
/// long queue is not pestered.
const QUEUE_POSITION_INTERVAL: Duration = Duration::from_mins(5);

/// How long a write to a child may block. The relay runs under the client
/// context lock, so an unbounded write would freeze every other operation
/// behind one child that stopped reading.
const CHILD_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

impl Client {
    pub(crate) fn listen_to_client_operations(
        reader: Receiver<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        thread::spawn(move || {
            let mut last_sweep = Instant::now();
            let mut last_queue_poll = Instant::now();
            loop {
                let next = reader.recv_timeout(CONNECT_SWEEP_INTERVAL);
                if last_sweep.elapsed() >= CONNECT_SWEEP_INTERVAL {
                    last_sweep = Instant::now();
                    Self::sweep_expired_connects(&client_context);
                    Self::sweep_stale_offers(&client_context);
                    if let Ok(mut ctx) = client_context.write_safe() {
                        ctx.expire_pending_peer_messages(Instant::now());
                        if let Some(branch) =
                            ctx.leaf.due_announcement(Instant::now())
                        {
                            super::distributed::announce_move(
                                &mut ctx, &branch, true,
                            );
                        }
                    }
                }
                if last_queue_poll.elapsed() >= QUEUE_POSITION_INTERVAL {
                    last_queue_poll = Instant::now();
                    Self::poll_queue_positions(&client_context);
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
                    operation @ (ClientOperation::PeerDisconnected(..)
                    | ClientOperation::PeerConnectFailed(..)
                    | ClientOperation::ChildConnected { .. }) => {
                        Self::on_peers(
                            operation,
                            &client_context,
                            &own_username,
                        );
                    }
                    operation @ (ClientOperation::DownloadFromPeer(..)
                    | ClientOperation::UpdateDownloadTokens(..)
                    | ClientOperation::StartUpload { .. }
                    | ClientOperation::QueueUpload { .. }) => {
                        Self::on_transfers(
                            operation,
                            &client_context,
                            &own_username,
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
                    } => {
                        // Stats about ourselves carry the speed the server
                        // records for us — pushed whenever it changes, since
                        // we watch ourselves — and the distributed child
                        // limit is derived from it.
                        let own = username == own_username;
                        match client_context.write_safe() {
                            Ok(mut ctx) => {
                                ctx.apply_user_stats(
                                    username,
                                    average_speed,
                                    shared_files,
                                    shared_folders,
                                );
                                if own {
                                    ctx.set_own_average_speed(average_speed);
                                }
                            }
                            Err(e) => {
                                error!(
                                    "[client] UserStatsReceived write: {}",
                                    e
                                );
                            }
                        }
                        if own {
                            Self::announce_child_capacity(&client_context);
                        }
                    }
                    ClientOperation::WatchedUserReceived {
                        username,
                        exists,
                        status,
                        average_speed,
                        shared_files,
                        shared_folders,
                    } => {
                        let own = username == own_username;
                        match client_context.write_safe() {
                            Ok(mut ctx) => {
                                ctx.apply_watched_user(
                                    username,
                                    exists,
                                    status,
                                    average_speed,
                                    shared_files,
                                    shared_folders,
                                );
                                // Watching ourselves is how our own recorded
                                // speed reaches us, and the child limit is
                                // derived from it.
                                if own && let Some(speed) = average_speed {
                                    ctx.set_own_average_speed(speed);
                                }
                            }
                            Err(e) => {
                                error!(
                                    "[client] WatchedUserReceived write: {}",
                                    e
                                );
                            }
                        }
                        if own {
                            Self::announce_child_capacity(&client_context);
                        }
                    }
                    ClientOperation::RoomEvent(event) => {
                        match client_context.write_safe() {
                            Ok(mut ctx) => ctx.apply_room_event(event),
                            Err(e) => error!("[client] RoomEvent write: {}", e),
                        }
                    }
                    ClientOperation::RoomMemberStats { room, stats } => {
                        match client_context.write_safe() {
                            Ok(mut ctx) => {
                                ctx.apply_room_member_stats(room, stats);
                            }
                            Err(e) => {
                                error!("[client] RoomMemberStats write: {}", e);
                            }
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
                    operation @ ClientOperation::IncomingSearch { .. } => {
                        Self::on_search(
                            operation,
                            &client_context,
                            &own_username,
                        );
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
                    ClientOperation::PossibleParents(candidates) => {
                        let (dials, ops) = match client_context.write_safe() {
                            Ok(mut ctx) => (
                                ctx.leaf.consider(candidates),
                                ctx.operations.clone(),
                            ),
                            Err(_) => continue,
                        };
                        let Some(ops) = ops else { continue };
                        for dial in dials {
                            super::distributed::spawn_link(
                                dial,
                                own_username.clone(),
                                ops.clone(),
                            );
                        }
                    }
                    operation @ (ClientOperation::ParentSearch { .. }
                    | ClientOperation::ParentBranchLevel {
                        ..
                    }
                    | ClientOperation::ParentBranchRoot {
                        ..
                    }
                    | ClientOperation::ParentClosed { .. }
                    | ClientOperation::ResetDistributed) => {
                        Self::on_tree(operation, &client_context);
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
                    ClientOperation::Recommendations {
                        global,
                        recommended,
                        unrecommended,
                    } => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.apply_recommendations(
                                global,
                                recommended,
                                unrecommended,
                            );
                        }
                    }
                    ClientOperation::ItemRecommendations {
                        item,
                        recommendations,
                    } => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.apply_item_recommendations(
                                item,
                                recommendations,
                            );
                        }
                    }
                    ClientOperation::SimilarUsers(users) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.apply_similar_users(users);
                        }
                    }
                    ClientOperation::ItemSimilarUsers { item, usernames } => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.apply_item_similar_users(item, usernames);
                        }
                    }
                    ClientOperation::UserInterests(interests) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.apply_user_interests(interests);
                        }
                    }
                    ClientOperation::PeerInfoReceived { username, info } => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.store_peer_info(username, info);
                        }
                    }
                    ClientOperation::FolderContents {
                        username,
                        token,
                        folder,
                        directories,
                    } => {
                        trace!(
                            "[client] folder {} from {} (token {}): {} dirs",
                            folder,
                            username,
                            token,
                            directories.len()
                        );
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.store_folder_contents(
                                username,
                                folder,
                                directories,
                            );
                        }
                    }
                    ClientOperation::ParentMinSpeed(speed) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.set_parent_min_speed(speed);
                        }
                        Self::announce_child_capacity(&client_context);
                    }
                    ClientOperation::ParentSpeedRatio(ratio) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.set_parent_speed_ratio(ratio);
                        }
                        Self::announce_child_capacity(&client_context);
                    }
                    ClientOperation::SessionEstablished => {
                        // Interests live in the server only for the session
                        // that set them, so a new one starts by sending ours
                        // again — what Nicotine+ does from its config file.
                        let (sender, interests) = match client_context
                            .read_safe()
                        {
                            Ok(ctx) => {
                                (ctx.server_sender.clone(), ctx.own_interests())
                            }
                            Err(_) => continue,
                        };
                        let Some(sender) = sender else { continue };
                        // Watch ourselves: the server pushes a user's stats
                        // to whoever watches them, and our own recorded
                        // upload speed is what the distributed child limit is
                        // derived from. Nicotine+ reads the same figure from
                        // the stats the server sends about the logged-in
                        // user.
                        let _ = sender.send(ServerMessage::SendMessage(
                            MessageFactory::build_watch_user(&own_username),
                        ));
                        for item in &interests.likes {
                            let _ = sender.send(ServerMessage::SendMessage(
                                MessageFactory::build_add_thing_i_like(item),
                            ));
                        }
                        for item in &interests.hates {
                            let _ = sender.send(ServerMessage::SendMessage(
                                MessageFactory::build_add_thing_i_hate(item),
                            ));
                        }
                    }
                    ClientOperation::ExcludedSearchPhrases(phrases) => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.set_excluded_search_phrases(phrases);
                        }
                    }
                    ClientOperation::CantConnectToPeer { token } => {
                        // The peer we asked the server to broker gave up. The
                        // token is the correlation we sent, so it names the
                        // peer: end the wait now instead of letting every
                        // queued download for them sit until the timeout.
                        let username = match client_context.write_safe() {
                            Ok(mut ctx) => ctx.take_pending_connect(token),
                            Err(e) => {
                                error!(
                                    "[client] CantConnectToPeer write: {}",
                                    e
                                );
                                None
                            }
                        };
                        if let Some(username) = username {
                            debug!(
                                "[client] {} cannot connect back (token {})",
                                username, token
                            );
                            Self::fail_queued_downloads(
                                &client_context,
                                &username,
                            );
                        }
                    }
                    ClientOperation::ShareListRequested { requester_key } => {
                        Self::reply_to_peer(
                            &client_context,
                            &requester_key,
                            |ctx| {
                                crate::message::peer::build_shared_file_list(
                                    &ctx.shares.directories(),
                                )
                            },
                        );
                    }
                    ClientOperation::UserInfoRequested { requester_key } => {
                        Self::reply_to_peer(
                            &client_context,
                            &requester_key,
                            |ctx| {
                                crate::message::peer::build_user_info(
                                    ctx.upload_slots as u32,
                                    ctx.upload_queue.len() as u32,
                                    ctx.has_free_upload_slot(),
                                )
                            },
                        );
                    }
                    ClientOperation::FolderContentsRequested {
                        requester_key,
                        token,
                        folder,
                    } => {
                        // ponytail: walks the whole share for one folder, as a
                        // browse does; index by directory if it ever shows up.
                        Self::reply_to_peer(
                            &client_context,
                            &requester_key,
                            |ctx| {
                                let dirs: Vec<_> = ctx
                                    .shares
                                    .directories()
                                    .into_iter()
                                    .filter(|dir| dir.name == folder)
                                    .collect();
                                crate::message::peer::build_folder_contents(
                                    token, &folder, &dirs,
                                )
                            },
                        );
                    }
                    ClientOperation::BrowseResult {
                        username,
                        directories,
                    } => {
                        if let Ok(mut ctx) = client_context.write_safe() {
                            ctx.store_browse_result(username, directories);
                        }
                    }
                }
            }
        });
    }

    /// Ask every peer holding a queued download of ours where it sits. The
    /// answers arrive as `PlaceInQueueResponse` and land on the downloads.
    fn poll_queue_positions(client_context: &Arc<RwLock<ClientContext>>) {
        let (registry, queued) = match client_context.read_safe() {
            Ok(ctx) => (
                ctx.peer_registry.clone(),
                ctx.get_downloads()
                    .iter()
                    .filter(|d| matches!(d.status, DownloadStatus::Queued))
                    .map(|d| (d.username.clone(), d.filename.clone()))
                    .collect::<Vec<_>>(),
            ),
            Err(_) => return,
        };
        let Some(registry) = registry else { return };
        for (username, filename) in queued {
            // Only over a connection we already hold: a peer we cannot reach
            // is a connection problem, not a queue question.
            if !registry.contains(&username) {
                continue;
            }
            let request =
                MessageFactory::build_place_in_queue_request(&filename);
            let _ = registry
                .send_to_peer(&username, PeerMessage::SendMessage(request));
        }
    }

    /// Tell the server whether we would take another child right now
    /// (`AcceptChildren`, code 100). Sent whenever that answer changes: the
    /// server only offers us to peers while it is true.
    fn announce_child_capacity(client_context: &Arc<RwLock<ClientContext>>) {
        let (sender, has_room) = match client_context.write_safe() {
            Ok(mut ctx) => {
                // Only on a change: the server holds this as a standing
                // state, and repeating it on every message we happen to
                // receive would be noise.
                let Some(has_room) = ctx.accept_children_change() else {
                    return;
                };
                (ctx.server_sender.clone(), has_room)
            }
            Err(_) => return,
        };
        let Some(sender) = sender else { return };
        let message =
            crate::message::server::MessageFactory::build_accept_children(
                has_room,
            );
        let _ = sender.send(ServerMessage::SendMessage(message));
    }

    /// Answer a peer with the message `build` derives from the client state.
    fn reply_to_peer(
        client_context: &Arc<RwLock<ClientContext>>,
        requester_key: &str,
        build: impl FnOnce(&ClientContext) -> crate::message::Message,
    ) {
        let (registry, message) = match client_context.read_safe() {
            Ok(ctx) => (ctx.peer_registry.clone(), build(&ctx)),
            Err(_) => return,
        };
        if let Some(registry) = registry {
            let _ = registry
                .send_to_peer(requester_key, PeerMessage::SendMessage(message));
        }
    }

    fn sweep_stale_offers(client_context: &Arc<RwLock<ClientContext>>) {
        let freed = client_context
            .write_safe()
            .is_ok_and(|mut ctx| ctx.expire_stale_offers(Instant::now()));
        if freed {
            Self::pump_upload_queue(client_context);
        }
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
