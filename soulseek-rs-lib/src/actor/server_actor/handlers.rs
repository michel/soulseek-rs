//! What the actor does with a message once it has one: the dispatch and the
//! per-message handlers behind it.
//!
//! A child module, so these keep reaching the actor's private state without
//! any of it being widened for the move.

use super::{
    ClientOperation, ClientVersion, ConnectionState, ConnectionType, Duration,
    LOGIN_VERDICT_TIMEOUT, MessageFactory, Peer, RoomEvent, RwLockExt,
    ServerActor, ServerMessage, SoulseekRs, UserMessage, debug, error,
    post_login_messages,
};

impl ServerActor {
    pub(super) fn handle_message(&mut self, msg: ServerMessage) {
        if !matches!(self.connection_state, ConnectionState::Connected)
            && !matches!(
                &msg,
                ServerMessage::ProcessRead | ServerMessage::Login { .. }
            )
        {
            self.queued_messages.push(msg);
            return;
        }

        match msg {
            ServerMessage::ConnectToPeer(peer) => {
                self.handle_connect_to_peer(peer);
            }
            ServerMessage::LoginStatus(message) => {
                self.handle_login_status(message);
            }
            ServerMessage::Relogged => self.handle_relogged(),
            ServerMessage::PierceFirewall(token) => {
                self.send_message(
                    MessageFactory::build_pierce_firewall_message(token),
                );
            }
            ServerMessage::SendMessage(message) => {
                self.send_message(message);
            }
            ServerMessage::GetPeerAddress(username) => {
                self.send_message(MessageFactory::build_get_peer_address(
                    &username,
                ));
            }
            ServerMessage::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            } => {
                self.handle_get_peer_address_response(
                    username,
                    host,
                    port,
                    obfuscation_type,
                    obfuscated_port,
                );
            }
            ServerMessage::PrivateMessageReceived(user_message) => {
                self.handle_private_message_received(user_message);
            }
            ServerMessage::ProcessRead => {
                self.process_read();
            }
            ServerMessage::Login {
                username,
                password,
                version,
                response,
            } => {
                self.handle_login(username, password, version, response);
            }
            ServerMessage::FileSearch { token, query } => {
                self.file_search(token, &query);
            }
            ServerMessage::FileSearchRequest {
                username,
                token,
                query,
            } => {
                self.handle_file_search_request(username, token, query);
            }
            other => self.handle_social_message(other),
        }
    }

    /// Rooms and users (codes 7, 5, 36, 13-17, 64): forwarded as room events
    /// and user status or stats. Split out, like the standing traffic below,
    /// only to keep `handle_message`'s match readable.
    fn handle_social_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::RoomListReceived(rooms) => {
                self.forward_room_event(RoomEvent::List(rooms));
            }
            ServerMessage::UserStatusReceived {
                username,
                status,
                privileged,
            } => {
                self.forward_to_client(ClientOperation::UserStatusReceived {
                    username,
                    status,
                    privileged,
                });
            }
            ServerMessage::WatchedUserReceived {
                username,
                exists,
                status,
                average_speed,
                shared_files,
                shared_folders,
            } => {
                self.forward_to_client(ClientOperation::WatchedUserReceived {
                    username,
                    exists,
                    status,
                    average_speed,
                    shared_files,
                    shared_folders,
                });
            }
            ServerMessage::UserStatsReceived {
                username,
                average_speed,
                shared_files,
                shared_folders,
            } => {
                self.forward_to_client(ClientOperation::UserStatsReceived {
                    username,
                    average_speed,
                    shared_files,
                    shared_folders,
                });
            }
            ServerMessage::RoomJoined { room, users } => {
                self.forward_room_event(RoomEvent::Joined { room, users });
            }
            ServerMessage::RoomMemberStats { room, stats } => {
                self.forward_to_client(ClientOperation::RoomMemberStats {
                    room,
                    stats,
                });
            }
            ServerMessage::RoomLeft { room } => {
                self.forward_room_event(RoomEvent::Left { room });
            }
            ServerMessage::RoomMessageReceived {
                room,
                username,
                message,
            } => {
                self.forward_room_event(RoomEvent::Message {
                    room,
                    username,
                    message,
                });
            }
            ServerMessage::RoomUserJoined { room, username } => {
                self.forward_room_event(RoomEvent::UserJoined {
                    room,
                    username,
                });
            }
            ServerMessage::RoomUserLeft { room, username } => {
                self.forward_room_event(RoomEvent::UserLeft { room, username });
            }
            ServerMessage::RoomTickers { room, tickers } => {
                self.forward_room_event(RoomEvent::Tickers { room, tickers });
            }
            ServerMessage::RoomTickerAdded {
                room,
                username,
                ticker,
            } => {
                self.forward_room_event(RoomEvent::TickerAdded {
                    room,
                    username,
                    ticker,
                });
            }
            ServerMessage::RoomTickerRemoved { room, username } => {
                self.forward_room_event(RoomEvent::TickerRemoved {
                    room,
                    username,
                });
            }
            ServerMessage::GlobalRoomMessageReceived {
                room,
                username,
                message,
            } => {
                self.forward_room_event(RoomEvent::GlobalMessage {
                    room,
                    username,
                    message,
                });
            }
            other => self.handle_standing_message(other),
        }
    }

    /// The wishlist and privilege traffic: standing searches (codes 103/104) and
    /// who is privileged (codes 69/92).
    ///
    /// Split out only because it keeps `handle_message`'s match to a readable
    /// length; there is no behaviour here beyond dispatch.
    fn handle_standing_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::PossibleParents(candidates) => {
                self.forward_to_client(ClientOperation::PossibleParents(
                    candidates,
                ));
            }
            ServerMessage::ResetDistributed => {
                self.forward_to_client(ClientOperation::ResetDistributed);
            }
            ServerMessage::WishlistSearch { token, query } => {
                self.queue_message(MessageFactory::build_wishlist_search(
                    token, &query,
                ));
            }
            ServerMessage::WishlistInterval(seconds) => {
                self.forward_to_client(ClientOperation::WishlistInterval(
                    seconds,
                ));
            }
            ServerMessage::PrivilegedUsers(users) => {
                self.forward_to_client(ClientOperation::PrivilegedUsers(users));
            }
            ServerMessage::OwnPrivileges(seconds) => {
                self.forward_to_client(ClientOperation::OwnPrivileges(seconds));
            }
            ServerMessage::CheckPrivileges => {
                self.queue_message(MessageFactory::build_check_privileges());
            }
            message @ (ServerMessage::RecommendationsReceived { .. }
            | ServerMessage::GlobalRecommendationsReceived {
                ..
            }
            | ServerMessage::ItemRecommendationsReceived {
                ..
            }
            | ServerMessage::SimilarUsersReceived { .. }
            | ServerMessage::ItemSimilarUsersReceived { .. }
            | ServerMessage::UserInterestsReceived(..)) => {
                self.handle_taste_message(message);
            }
            ServerMessage::CantConnectToPeer { token } => {
                self.forward_to_client(ClientOperation::CantConnectToPeer {
                    token,
                });
            }
            ServerMessage::PrivateRoomMembers { room, users } => {
                self.forward_room_event(RoomEvent::PrivateMembers {
                    room,
                    users,
                });
            }
            ServerMessage::PrivateRoomOperators { room, users } => {
                self.forward_room_event(RoomEvent::PrivateOperators {
                    room,
                    users,
                });
            }
            ServerMessage::PrivateRoomRosterChanged {
                room,
                username,
                members,
                added,
            } => {
                self.forward_room_event(RoomEvent::PrivateRosterChanged {
                    room,
                    username,
                    members,
                    added,
                });
            }
            ServerMessage::OwnRoomStandingChanged {
                room,
                members,
                granted,
            } => {
                self.forward_room_event(RoomEvent::OwnStandingChanged {
                    room,
                    members,
                    granted,
                });
            }
            ServerMessage::CantCreateRoom { room } => {
                self.forward_room_event(RoomEvent::CantCreate { room });
            }
            ServerMessage::ParentMinSpeed(speed) => {
                self.forward_to_client(ClientOperation::ParentMinSpeed(speed));
            }
            ServerMessage::ParentSpeedRatio(ratio) => {
                self.forward_to_client(ClientOperation::ParentSpeedRatio(
                    ratio,
                ));
            }
            ServerMessage::ExcludedSearchPhrases(phrases) => {
                self.forward_to_client(ClientOperation::ExcludedSearchPhrases(
                    phrases,
                ));
            }
            other => {
                error!("[server] unroutable message: {:?}", other);
            }
        }
    }

    /// What the server says about taste: the recommendation and similar-user
    /// answers, and a user's interests. Split off `handle_standing_message`
    /// only to keep that dispatch readable — these arms share nothing with
    /// the rest of it.
    fn handle_taste_message(&self, message: ServerMessage) {
        match message {
            ServerMessage::RecommendationsReceived {
                recommended,
                unrecommended,
            } => {
                self.forward_to_client(ClientOperation::Recommendations {
                    global: false,
                    recommended,
                    unrecommended,
                });
            }
            ServerMessage::GlobalRecommendationsReceived {
                recommended,
                unrecommended,
            } => {
                self.forward_to_client(ClientOperation::Recommendations {
                    global: true,
                    recommended,
                    unrecommended,
                });
            }
            ServerMessage::ItemRecommendationsReceived {
                item,
                recommendations,
            } => {
                self.forward_to_client(ClientOperation::ItemRecommendations {
                    item,
                    recommendations,
                });
            }
            ServerMessage::SimilarUsersReceived { users } => {
                self.forward_to_client(ClientOperation::SimilarUsers(users));
            }
            ServerMessage::ItemSimilarUsersReceived { item, usernames } => {
                self.forward_to_client(ClientOperation::ItemSimilarUsers {
                    item,
                    usernames,
                });
            }
            ServerMessage::UserInterestsReceived(interests) => {
                self.forward_to_client(ClientOperation::UserInterests(
                    interests,
                ));
            }
            // `handle_standing_message` routes only the variants above here.
            _ => {}
        }
    }

    fn handle_connect_to_peer(&self, peer: Peer) {
        if let Some(op) = match peer.connection_type {
            ConnectionType::P | ConnectionType::F => {
                Some(ClientOperation::ConnectToPeer(peer))
            }
            ConnectionType::D => None,
        } && let Err(e) = self.client_channel.send(op)
        {
            error!("[server] failed to send ConnectToPeer: {}", e);
        }
    }

    pub(super) fn handle_login_status(&mut self, message: bool) {
        // Send the post-login handshake exactly once, only on success,
        // on the live path (the old ServerActor::login did this but was
        // never called). Advertises real shared counts and, when
        // listening, the port peers must connect to.
        if message {
            for msg in post_login_messages(
                self.enable_listen,
                self.listen_port,
                self.shared_folder_count,
                self.shared_file_count,
            ) {
                self.send_message(msg);
            }
            self.session.clear();
            // The distributed stance is the leaf's to announce, and a new
            // session starts without a parent.
            self.forward_to_client(ClientOperation::ResetDistributed);
            // The server holds interests only for the life of a session.
            self.forward_to_client(ClientOperation::SessionEstablished);
        }
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.logged_in = Some(message),
            Err(e) => {
                error!("[server] LoginStatus write: {}", e);
            }
        }
    }

    fn handle_get_peer_address_response(
        &self,
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    ) {
        debug!(
            "[server] Received GetPeerAddress response for {}: {}:{} (obf_type: {}, obf_port: {})",
            username, host, port, obfuscation_type, obfuscated_port
        );

        if let Err(e) =
            self.client_channel
                .send(ClientOperation::GetPeerAddressResponse {
                    username,
                    host,
                    port,
                    obfuscation_type,
                    obfuscated_port,
                })
        {
            error!(
                "[server] Error forwarding GetPeerAddress response to client: {}",
                e
            );
        }
    }

    /// Hand an operation to the client loop, logging a dead channel rather
    /// than unwinding the actor.
    fn forward_to_client(&self, operation: ClientOperation) {
        if let Err(e) = self.client_channel.send(operation) {
            error!("[server] Error forwarding to client: {}", e);
        }
    }

    fn handle_private_message_received(&self, user_message: UserMessage) {
        debug!("[server] Private message from {}", user_message.username());
        if let Err(e) = self
            .client_channel
            .send(ClientOperation::PrivateMessageReceived(user_message))
        {
            error!(
                "[server] Error forwarding private message to client: {}",
                e
            );
        }
    }

    pub(super) fn handle_login(
        &mut self,
        username: String,
        password: String,
        version: ClientVersion,
        response: std::sync::mpsc::Sender<Result<bool, SoulseekRs>>,
    ) {
        if self.stream.is_none() && !self.initiate_connection() {
            let _ = response.send(Err(SoulseekRs::NotConnected));
            return;
        }
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.logged_in = None;
        }
        self.queue_message(MessageFactory::build_login_message(
            &username, &password, version,
        ));

        let start = std::time::Instant::now();

        let context = self.context.clone();
        std::thread::spawn(move || {
            loop {
                if start.elapsed() >= LOGIN_VERDICT_TIMEOUT {
                    let _ = response.send(Err(SoulseekRs::Timeout));
                    break;
                }

                let logged_in = match context.read_safe() {
                    Ok(ctx) => ctx.logged_in,
                    Err(e) => {
                        let _ = response.send(Err(e));
                        break;
                    }
                };
                if let Some(logged_in) = logged_in {
                    let result = if logged_in {
                        Ok(true)
                    } else {
                        Err(SoulseekRs::AuthenticationFailed)
                    };
                    let _ = response.send(result);
                    break;
                }

                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }

    fn handle_file_search_request(
        &self,
        username: String,
        token: u32,
        query: String,
    ) {
        if let Err(e) =
            self.client_channel.send(ClientOperation::IncomingSearch {
                from_parent: false,
                username,
                token,
                query,
            })
        {
            error!("[server] forward IncomingSearch: {}", e);
        }
    }
}
