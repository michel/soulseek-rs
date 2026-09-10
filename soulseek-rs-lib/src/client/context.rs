//! What the client's shared context does, as opposed to what it holds.
//!
//! A child module rather than a sibling: the state itself stays declared in
//! `client`, so these methods keep reaching its private fields without any
//! of them having to be widened for the move.

use super::{
    ActorSystem, Arc, AtomicUsize, BROKER_CONNECT_TIMEOUT,
    BROWSE_PROTECT_WINDOW, ClientContext, DEFAULT_MAX_PEERS,
    DEFAULT_UPLOAD_SLOTS, DownloadStatus, DownloadStore, HashMap, HashSet,
    Instant, PENDING_PEER_TTL, Recommendation, RoomEvent, RoomInfo, RoomTicker,
    RoomUserStats, SharedDirectory, Shares, SimilarUser, UserInfo,
    UserInterests, UserMessage, UserPresence, UserStats, UserStatus, children,
    distributed, sorted_unique,
};

impl ClientContext {
    /// A context for the client that logs in as `username`.
    #[must_use]
    pub fn for_user(username: &str) -> Self {
        Self {
            leaf: distributed::Leaf::new(username),
            ..Self::new()
        }
    }

    #[must_use]
    pub fn new() -> Self {
        let actor_system = Arc::new(ActorSystem::new());

        Self {
            peer_registry: None,
            server_sender: None,
            operations: None,
            leaf: distributed::Leaf::new(""),
            children: children::Children::default(),
            announced_accept_children: None,
            own_average_speed: None,
            parent_min_speed: 0,
            parent_speed_ratio: 0,
            searches: HashMap::new(),
            private_messages: Vec::new(),
            pending_connect_tokens: HashMap::new(),
            max_peers: Arc::new(AtomicUsize::new(DEFAULT_MAX_PEERS)),
            shares: Arc::new(Shares::empty()),
            shared_directories: Vec::new(),
            peer_addresses: HashMap::new(),
            pending_peer_messages: HashMap::new(),
            uploads: HashMap::new(),
            active_uploads: HashMap::new(),
            pending_serves: HashMap::new(),
            browse_results: HashMap::new(),
            folder_contents: HashMap::new(),
            peer_infos: HashMap::new(),
            pending_browses: HashMap::new(),
            room_list: Vec::new(),
            room_events: Vec::new(),
            room_members: HashMap::new(),
            room_member_stats: HashMap::new(),
            room_tickers: HashMap::new(),
            private_room_members: HashMap::new(),
            private_room_operators: HashMap::new(),
            recommendations: None,
            global_recommendations: None,
            item_recommendations: HashMap::new(),
            similar_users: Vec::new(),
            item_similar_users: HashMap::new(),
            user_interests: HashMap::new(),
            own_interests: UserInterests::default(),
            user_info: HashMap::new(),
            watched_users: HashSet::new(),
            wishlist_interval: None,
            privileged_users: HashSet::new(),
            own_privileges: None,
            excluded_search_phrases: Vec::new(),
            upload_queue: Vec::new(),
            upload_seq: 0,
            upload_slots: DEFAULT_UPLOAD_SLOTS,
            last_upload_speed: 0,
            upload_events: Vec::new(),
            downloads: DownloadStore::new(),
            actor_system,
        }
    }

    /// Apply a chat-room event: keep the room-list snapshot and the per-room
    /// rosters current, then queue the event for the client/UI to drain.
    pub fn apply_room_event(&mut self, event: RoomEvent) {
        match &event {
            RoomEvent::List(rooms) => self.room_list.clone_from(rooms),
            RoomEvent::Joined { room, users } => {
                let mut members = users.clone();
                members.sort();
                members.dedup();
                self.room_members.insert(room.clone(), members);
            }
            RoomEvent::Left { room } => {
                self.room_members.remove(room);
                // A board for a room we are not in is stale; the server sends
                // the whole board again on the next join.
                self.room_tickers.remove(room);
            }
            RoomEvent::UserJoined { room, username } => {
                let members =
                    self.room_members.entry(room.clone()).or_default();
                if let Err(at) = members.binary_search(username) {
                    members.insert(at, username.clone());
                }
            }
            RoomEvent::UserLeft { room, username } => {
                if let Some(members) = self.room_members.get_mut(room)
                    && let Ok(at) = members.binary_search(username)
                {
                    members.remove(at);
                }
            }
            RoomEvent::Tickers { room, tickers } => {
                self.room_tickers.insert(room.clone(), tickers.clone());
            }
            RoomEvent::TickerAdded {
                room,
                username,
                ticker,
            } => {
                // A user has at most one ticker: an added one replaces theirs
                // rather than stacking, which is how the server treats it.
                let board = self.room_tickers.entry(room.clone()).or_default();
                board.retain(|t| &t.username != username);
                board.push(RoomTicker {
                    username: username.clone(),
                    ticker: ticker.clone(),
                });
            }
            RoomEvent::TickerRemoved { room, username } => {
                if let Some(board) = self.room_tickers.get_mut(room) {
                    board.retain(|t| &t.username != username);
                }
            }
            RoomEvent::PrivateMembers { room, users } => {
                self.private_room_members
                    .insert(room.clone(), sorted_unique(users));
            }
            RoomEvent::PrivateOperators { room, users } => {
                self.private_room_operators
                    .insert(room.clone(), sorted_unique(users));
            }
            RoomEvent::PrivateRosterChanged {
                room,
                username,
                members,
                added,
            } => {
                let roster = if *members {
                    self.private_room_members.entry(room.clone()).or_default()
                } else {
                    self.private_room_operators.entry(room.clone()).or_default()
                };
                match (added, roster.binary_search(username)) {
                    (true, Err(at)) => roster.insert(at, username.clone()),
                    (false, Ok(at)) => {
                        roster.remove(at);
                    }
                    _ => {}
                }
            }
            RoomEvent::OwnStandingChanged {
                room,
                members,
                granted,
            } => {
                // Losing membership takes both rosters with it: a room we
                // cannot enter has none worth showing. Losing operatorship
                // leaves us a member who still sees them, and the server
                // narrates that roster change separately (code 144).
                if !granted && *members {
                    self.private_room_members.remove(room);
                    self.private_room_operators.remove(room);
                }
            }
            RoomEvent::Message { .. }
            | RoomEvent::GlobalMessage { .. }
            | RoomEvent::CantCreate { .. } => {}
        }
        self.room_events.push(event);
    }

    /// Remember an interest of our own so it can be sent again next login.
    /// Interests are matched by the server case-insensitively, and Nicotine+
    /// lowercases them before sending; do the same so two spellings of one
    /// interest cannot both be held.
    /// Returns the stored spelling, or `None` for an empty item — which is
    /// no interest, and must not be sent to the server either.
    pub fn add_own_interest(
        &mut self,
        item: &str,
        liked: bool,
    ) -> Option<String> {
        let item = item.trim().to_lowercase();
        if item.is_empty() {
            return None;
        }
        let list = if liked {
            &mut self.own_interests.likes
        } else {
            &mut self.own_interests.hates
        };
        if !list.contains(&item) {
            list.push(item.clone());
        }
        Some(item)
    }

    /// Forget one of our own interests. `None` for an empty item, as above.
    pub fn remove_own_interest(
        &mut self,
        item: &str,
        liked: bool,
    ) -> Option<String> {
        let item = item.trim().to_lowercase();
        if item.is_empty() {
            return None;
        }
        let list = if liked {
            &mut self.own_interests.likes
        } else {
            &mut self.own_interests.hates
        };
        list.retain(|held| held != &item);
        Some(item)
    }

    /// What we like and hate, as last set.
    #[must_use]
    pub fn own_interests(&self) -> UserInterests {
        self.own_interests.clone()
    }

    /// Take the `AcceptChildren` answer to send, or `None` when the server
    /// already has this one.
    pub fn accept_children_change(&mut self) -> Option<bool> {
        let has_room = self.children.has_room();
        (self.announced_accept_children != Some(has_room)).then(|| {
            self.announced_accept_children = Some(has_room);
            has_room
        })
    }

    /// The upload speed the server records for us, if it has said.
    #[must_use]
    pub const fn own_average_speed(&self) -> Option<u32> {
        self.own_average_speed
    }

    /// Record what the server says our own upload speed is, and re-derive the
    /// child limit from it.
    pub fn set_own_average_speed(&mut self, speed: u32) {
        self.own_average_speed = Some(speed);
        self.apply_child_limits();
    }

    /// Record the server's parent figures (codes 83 and 84).
    pub fn set_parent_min_speed(&mut self, speed: u32) {
        self.parent_min_speed = speed;
        self.apply_child_limits();
    }

    pub fn set_parent_speed_ratio(&mut self, ratio: u32) {
        self.parent_speed_ratio = ratio;
        self.apply_child_limits();
    }

    fn apply_child_limits(&mut self) {
        self.children.set_limits(
            self.own_average_speed,
            self.parent_min_speed,
            self.parent_speed_ratio,
        );
    }

    /// Whether something is feeding us the distributed search stream: a
    /// parent of our own, or the server relaying searches to us. Children may
    /// only hang from a client that has one.
    pub fn set_fed_by(&mut self, fed: bool) {
        self.children.set_fed(fed);
        if !fed {
            self.children.drop_all();
        }
    }

    /// Record what `username` says about itself.
    pub fn store_peer_info(
        &mut self,
        username: String,
        info: crate::message::peer::PeerInfo,
    ) {
        self.peer_infos.insert(username, info);
    }

    /// What `username` last said about itself, if anything.
    #[must_use]
    pub fn peer_info(
        &self,
        username: &str,
    ) -> Option<crate::message::peer::PeerInfo> {
        self.peer_infos.get(username).cloned()
    }

    /// Drop what we hold about `username`, so a poll after a fresh request
    /// cannot report the previous answer.
    pub fn invalidate_peer_info(&mut self, username: &str) {
        self.peer_infos.remove(username);
    }

    /// Record one folder's listing from `username`.
    pub fn store_folder_contents(
        &mut self,
        username: String,
        folder: String,
        directories: Vec<SharedDirectory>,
    ) {
        self.folder_contents.insert((username, folder), directories);
    }

    /// Remove and return the listing of `folder` from `username`, if it has
    /// arrived.
    pub fn take_folder_contents(
        &mut self,
        username: &str,
        folder: &str,
    ) -> Option<Vec<SharedDirectory>> {
        self.folder_contents
            .remove(&(username.to_string(), folder.to_string()))
    }

    /// Who may enter the private room `room`, as last reported.
    #[must_use]
    pub fn private_room_members(&self, room: &str) -> Vec<String> {
        self.private_room_members
            .get(room)
            .cloned()
            .unwrap_or_default()
    }

    /// Who runs the private room `room`, as last reported.
    #[must_use]
    pub fn private_room_operators(&self, room: &str) -> Vec<String> {
        self.private_room_operators
            .get(room)
            .cloned()
            .unwrap_or_default()
    }

    /// The private rooms we belong to, in a stable order.
    #[must_use]
    pub fn private_rooms(&self) -> Vec<String> {
        let mut rooms: Vec<String> =
            self.private_room_members.keys().cloned().collect();
        rooms.sort();
        rooms
    }

    /// The ticker board of `room` as last reported by the server.
    #[must_use]
    pub fn room_tickers(&self, room: &str) -> Vec<RoomTicker> {
        self.room_tickers.get(room).cloned().unwrap_or_default()
    }

    /// Record a recommendations reply; `global` selects the server-wide set.
    pub fn apply_recommendations(
        &mut self,
        global: bool,
        recommended: Vec<Recommendation>,
        unrecommended: Vec<Recommendation>,
    ) {
        let slot = if global {
            &mut self.global_recommendations
        } else {
            &mut self.recommendations
        };
        *slot = Some((recommended, unrecommended));
    }

    /// The last recommendations reply, or `None` until one arrives.
    #[must_use]
    pub fn recommendations(
        &self,
        global: bool,
    ) -> Option<(Vec<Recommendation>, Vec<Recommendation>)> {
        if global {
            self.global_recommendations.clone()
        } else {
            self.recommendations.clone()
        }
    }

    pub fn apply_item_recommendations(
        &mut self,
        item: String,
        recommendations: Vec<Recommendation>,
    ) {
        self.item_recommendations.insert(item, recommendations);
    }

    #[must_use]
    pub fn item_recommendations(&self, item: &str) -> Vec<Recommendation> {
        self.item_recommendations
            .get(item)
            .cloned()
            .unwrap_or_default()
    }

    pub fn apply_similar_users(&mut self, users: Vec<SimilarUser>) {
        self.similar_users = users;
    }

    #[must_use]
    pub fn similar_users(&self) -> Vec<SimilarUser> {
        self.similar_users.clone()
    }

    pub fn apply_item_similar_users(
        &mut self,
        item: String,
        usernames: Vec<String>,
    ) {
        self.item_similar_users.insert(item, usernames);
    }

    #[must_use]
    pub fn item_similar_users(&self, item: &str) -> Vec<String> {
        self.item_similar_users
            .get(item)
            .cloned()
            .unwrap_or_default()
    }

    pub fn apply_user_interests(&mut self, interests: UserInterests) {
        self.user_interests
            .insert(interests.username.clone(), interests);
    }

    #[must_use]
    pub fn user_interests(&self, username: &str) -> Option<UserInterests> {
        self.user_interests.get(username).cloned()
    }

    /// Record the phrases the server excludes (code 160), lowercased here so
    /// the per-file check is a plain `contains` on a lowercased path.
    pub fn set_excluded_search_phrases(&mut self, phrases: Vec<String>) {
        self.excluded_search_phrases =
            phrases.iter().map(|p| p.to_lowercase()).collect();
    }

    /// The phrases the server refuses to search for.
    #[must_use]
    pub fn excluded_search_phrases(&self) -> Vec<String> {
        self.excluded_search_phrases.clone()
    }

    /// Forget any interests cached for `username`, so a poll after a fresh
    /// request cannot mistake the previous answer for this one.
    pub fn invalidate_user_interests(&mut self, username: &str) {
        self.user_interests.remove(username);
    }

    /// Record a `GetUserStatus` reply, merging it with any statistics already
    /// received for that user.
    pub fn apply_user_status(
        &mut self,
        username: String,
        status: u32,
        privileged: bool,
    ) {
        self.user_info
            .entry(username.clone())
            .or_insert_with(|| UserInfo::pending(username))
            .presence = Some(UserPresence {
            status: UserStatus::from_code(status),
            privileged,
        });
    }

    /// Forget what we know about `username`, so the next poll reports the
    /// answer to the request being made now rather than the previous one.
    pub fn invalidate_user_info(&mut self, username: &str) {
        self.user_info.remove(username);
    }

    /// Record a `GetUserStats` reply, merging it with any status already
    /// received for that user.
    pub fn apply_user_stats(
        &mut self,
        username: String,
        average_speed: u32,
        shared_files: u32,
        shared_folders: u32,
    ) {
        self.user_info
            .entry(username.clone())
            .or_insert_with(|| UserInfo::pending(username))
            .stats = Some(UserStats {
            average_speed,
            shared_files,
            shared_folders,
        });
    }

    /// Record that we are now watching `username`.
    pub fn add_watched_user(&mut self, username: &str) {
        self.watched_users.insert(username.to_string());
    }

    /// Forget a watch, dropping any snapshot we held for that user so a later
    /// re-watch reports a fresh answer rather than a stale one.
    pub fn remove_watched_user(&mut self, username: &str) {
        self.watched_users.remove(username);
        self.user_info.remove(username);
    }

    /// Everyone we are currently watching, sorted by name.
    #[must_use]
    pub fn watched_users(&self) -> Vec<String> {
        let mut users: Vec<String> =
            self.watched_users.iter().cloned().collect();
        users.sort();
        users
    }

    /// Record the initial snapshot from a `WatchUser` reply. A username the
    /// server does not know carries no stats and is dropped from the watch
    /// list, since the server will never push anything for it.
    pub fn apply_watched_user(
        &mut self,
        username: String,
        exists: bool,
        status: Option<u32>,
        average_speed: Option<u32>,
        shared_files: Option<u32>,
        shared_folders: Option<u32>,
    ) {
        if !exists {
            self.watched_users.remove(&username);
            return;
        }
        if let Some(status) = status {
            let entry = self
                .user_info
                .entry(username.clone())
                .or_insert_with(|| UserInfo::pending(username.clone()));
            // The watch reply carries no privileged flag, so keep whatever a
            // previous GetUserStatus told us rather than asserting `false`.
            let privileged =
                entry.presence.as_ref().is_some_and(|p| p.privileged);
            entry.presence = Some(UserPresence {
                status: UserStatus::from_code(status),
                privileged,
            });
        }
        if let (Some(average_speed), Some(shared_files), Some(shared_folders)) =
            (average_speed, shared_files, shared_folders)
        {
            self.apply_user_stats(
                username,
                average_speed,
                shared_files,
                shared_folders,
            );
        }
    }

    /// What the server has said about `username` so far.
    #[must_use]
    pub fn user_info(&self, username: &str) -> Option<UserInfo> {
        self.user_info.get(username).cloned()
    }

    /// Who is currently in `room`, sorted, or empty when we are not in it.
    #[must_use]
    pub fn room_members(&self, room: &str) -> Vec<String> {
        self.room_members.get(room).cloned().unwrap_or_default()
    }

    /// Record the per-member statistics of a room we just joined, replacing
    /// any previous snapshot for it.
    pub fn apply_room_member_stats(
        &mut self,
        room: String,
        stats: Vec<RoomUserStats>,
    ) {
        self.room_member_stats.insert(room, stats);
    }

    /// What the server reported about the members of `room`, sorted by name,
    /// or empty for a room we have not joined.
    #[must_use]
    pub fn room_member_stats(&self, room: &str) -> Vec<RoomUserStats> {
        let mut stats = self
            .room_member_stats
            .get(room)
            .cloned()
            .unwrap_or_default();
        stats.sort_by(|a, b| a.username.cmp(&b.username));
        stats
    }

    /// The latest snapshot of the public chat-room list.
    #[must_use]
    pub fn room_list(&self) -> Vec<RoomInfo> {
        self.room_list.clone()
    }

    /// Remove and return all chat-room events received since the last call.
    #[must_use]
    pub fn take_room_events(&mut self) -> Vec<RoomEvent> {
        std::mem::take(&mut self.room_events)
    }

    /// Cache a peer's listen address learned from a GetPeerAddress response.
    pub fn cache_peer_address(
        &mut self,
        username: &str,
        host: String,
        port: u32,
    ) {
        self.peer_addresses
            .insert(username.to_string(), (host, port));
    }

    /// The cached listen address for `username`, if known.
    #[must_use]
    pub fn peer_address(&self, username: &str) -> Option<(String, u32)> {
        self.peer_addresses.get(username).cloned()
    }

    /// Queue a peer message to send once a control connection to `username` is up.
    pub fn queue_peer_message(
        &mut self,
        username: &str,
        message: crate::message::Message,
    ) {
        let entry = self
            .pending_peer_messages
            .entry(username.to_string())
            .or_insert_with(|| (Instant::now(), Vec::new()));
        entry.0 = Instant::now();
        entry.1.push(message);
    }

    /// Remove and return the messages queued for `username`.
    pub fn take_peer_messages(
        &mut self,
        username: &str,
    ) -> Vec<crate::message::Message> {
        self.pending_peer_messages
            .remove(username)
            .map(|(_, messages)| messages)
            .unwrap_or_default()
    }

    /// Drop peer messages nobody could deliver within
    /// [`PENDING_PEER_TTL`]: the searcher went offline, or never existed.
    pub(crate) fn expire_pending_peer_messages(&mut self, now: Instant) {
        self.pending_peer_messages.retain(|_, (since, _)| {
            now.duration_since(*since) < PENDING_PEER_TTL
        });
    }

    /// Store a shared-file listing received from browsing `username`.
    pub fn store_browse_result(
        &mut self,
        username: String,
        directories: Vec<SharedDirectory>,
    ) {
        self.pending_browses.remove(&username);
        self.browse_results.insert(username, directories);
    }

    pub(crate) fn mark_browse_pending(&mut self, username: &str) {
        let now = Instant::now();
        self.pending_browses.retain(|_, deadline| *deadline > now);
        self.pending_browses
            .insert(username.to_string(), now + BROWSE_PROTECT_WINDOW);
    }

    /// Remove and return the shared-file listing browsed from `username`.
    pub fn take_browse_result(
        &mut self,
        username: &str,
    ) -> Option<Vec<SharedDirectory>> {
        self.browse_results.remove(username)
    }

    /// Remember that a server-brokered connection to `username` is pending under
    /// `token`; the peer will quote it back in a PierceFirewall.
    pub fn add_pending_connect(&mut self, token: u32, username: String) {
        self.pending_connect_tokens
            .insert(token, (username, Instant::now() + BROKER_CONNECT_TIMEOUT));
    }

    /// Resolve and consume the peer expected for a brokered connection `token`.
    pub fn take_pending_connect(&mut self, token: u32) -> Option<String> {
        self.pending_connect_tokens
            .remove(&token)
            .map(|(username, _)| username)
    }

    pub fn take_expired_connects(&mut self, now: Instant) -> Vec<String> {
        let mut expired = Vec::new();
        self.pending_connect_tokens
            .retain(|_, (username, deadline)| {
                if now >= *deadline {
                    expired.push(std::mem::take(username));
                    return false;
                }
                true
            });
        expired
    }

    pub(crate) fn protected_peers(&self) -> HashSet<String> {
        let mut protected: HashSet<String> = self
            .downloads
            .list()
            .iter()
            .filter(|d| {
                matches!(
                    d.status,
                    DownloadStatus::Queued
                        | DownloadStatus::InProgress { .. }
                        | DownloadStatus::Paused { .. }
                )
            })
            .map(|d| d.username.clone())
            .collect();
        protected
            .extend(self.active_uploads.values().map(|u| u.username.clone()));
        protected.extend(self.uploads.values().map(|j| j.downloader.clone()));
        protected
            .extend(self.upload_queue.iter().map(|q| q.downloader.clone()));
        protected.extend(self.pending_serves.keys().cloned());
        protected.extend(self.pending_peer_messages.keys().cloned());
        let now = Instant::now();
        protected.extend(
            self.pending_browses
                .iter()
                .filter(|(_, deadline)| **deadline > now)
                .map(|(username, _)| username.clone()),
        );
        protected
    }

    /// Record a private message received from another user.
    pub fn push_private_message(&mut self, message: UserMessage) {
        self.private_messages.push(message);
    }

    /// Remove and return all buffered private messages.
    pub fn take_private_messages(&mut self) -> Vec<UserMessage> {
        std::mem::take(&mut self.private_messages)
    }
}
