//! The social side of the protocol: room tickers (codes 113-116), the global
//! room feed (150-152), interests and the recommendation queries built on them
//! (51/52/54/56/57/110/111/112/117/118), and the odds and ends a client owes
//! the server — a keepalive ping (32), gifting privileges (123) and changing
//! our password (142).

use super::{
    Client, Recommendation, Result, RoomTicker, RwLockExt, SimilarUser,
    UserInterests, error,
};
use crate::message::server::MessageFactory;

impl Client {
    /// Send a keepalive ping (server code 32). The server does not reply; the
    /// point is that a connection with nothing to say still has traffic on it,
    /// so a NAT or the server's idle reaper does not drop it.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn ping_server(&self) -> Result<()> {
        self.send_server_message(MessageFactory::build_server_ping())
    }

    /// Join a private room, creating it (and owning it) if the name is free.
    ///
    /// A name already taken by someone else's private room comes back as a
    /// [`crate::RoomEvent::CantCreate`] rather than a join.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn join_private_room(&self, room: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_join_room(room, true))
    }

    /// Invite `username` into the private room we own (code 134).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn add_room_member(&self, room: &str, username: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_add_room_member(
            room, username,
        ))
    }

    /// Remove `username` from the private room we own (code 135).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn remove_room_member(&self, room: &str, username: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_remove_room_member(
            room, username,
        ))
    }

    /// Give up our own membership of a private room (code 136).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn leave_private_room(&self, room: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_cancel_room_membership(
            room,
        ))
    }

    /// Give up ownership of a private room, which deletes it (code 137).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn disband_private_room(&self, room: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_cancel_room_ownership(
            room,
        ))
    }

    /// Say whether we accept private-room invitations (code 141).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn set_room_invitations_enabled(&self, enabled: bool) -> Result<()> {
        self.send_server_message(MessageFactory::build_enable_room_invitations(
            enabled,
        ))
    }

    /// Make `username` an operator of our private room (code 143).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn add_room_operator(&self, room: &str, username: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_add_room_operator(
            room, username,
        ))
    }

    /// Take operator status away from `username` (code 144).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn remove_room_operator(
        &self,
        room: &str,
        username: &str,
    ) -> Result<()> {
        self.send_server_message(MessageFactory::build_remove_room_operator(
            room, username,
        ))
    }

    /// Give up our own operator status in `room` (code 147).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn resign_room_operatorship(&self, room: &str) -> Result<()> {
        self.send_server_message(
            MessageFactory::build_cancel_room_operatorship(room),
        )
    }

    /// Who may enter the private room `room`, as the server last reported.
    #[must_use]
    pub fn private_room_members(&self, room: &str) -> Vec<String> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.private_room_members(room),
            Err(e) => {
                error!("[client] private_room_members: {}", e);
                Vec::new()
            }
        }
    }

    /// Who runs the private room `room`.
    #[must_use]
    pub fn private_room_operators(&self, room: &str) -> Vec<String> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.private_room_operators(room),
            Err(e) => {
                error!("[client] private_room_operators: {}", e);
                Vec::new()
            }
        }
    }

    /// The private rooms we belong to.
    #[must_use]
    pub fn private_rooms(&self) -> Vec<String> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.private_rooms(),
            Err(e) => {
                error!("[client] private_rooms: {}", e);
                Vec::new()
            }
        }
    }

    /// Set our online status (server code 28): away, or back to online.
    ///
    /// Other clients see it in their `GetUserStatus` and watch replies, and
    /// the server uses it when it decides who to hand a search to.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn set_away(&self, away: bool) -> Result<()> {
        let status = u32::from(!away) + 1; // away = 1, online = 2
        self.send_server_message(MessageFactory::build_set_status_message(
            status,
        ))
    }

    /// Ask `username` what it says about itself (peer code 15): its
    /// description, upload slots and queue length — a user's profile as other
    /// clients show it.
    ///
    /// Any previous answer is dropped first, so [`Client::peer_info`] reports
    /// the reply to this request rather than a stale one.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection,
    /// or [`crate::SoulseekRs::LockPoisoned`] if the client state is poisoned.
    pub fn request_peer_info(&self, username: &str) -> Result<()> {
        self.context.write_safe()?.invalidate_peer_info(username);
        let request = crate::message::peer::build_user_info_request();
        self.send_to_peer_or_queue(username, request)
    }

    /// What `username` last said about itself.
    #[must_use]
    pub fn peer_info(
        &self,
        username: &str,
    ) -> Option<crate::message::peer::PeerInfo> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.peer_info(username))
    }

    /// Ask `username` for the contents of one folder of their shares (peer
    /// code 36) — what "download folder" in other clients asks for, without
    /// pulling their whole listing.
    ///
    /// The answer arrives asynchronously; take it with
    /// [`Client::take_folder_contents`].
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection,
    /// or [`crate::SoulseekRs::LockPoisoned`] if the client state is poisoned.
    pub fn request_folder_contents(
        &self,
        username: &str,
        folder: &str,
    ) -> Result<()> {
        let request = crate::message::peer::build_folder_contents_request(
            crate::client::next_search_token(),
            folder,
        );
        self.send_to_peer_or_queue(username, request)
    }

    /// Remove and return the listing of `folder` from `username`, once their
    /// answer to [`Client::request_folder_contents`] has arrived.
    #[must_use]
    pub fn take_folder_contents(
        &self,
        username: &str,
        folder: &str,
    ) -> Option<Vec<crate::SharedDirectory>> {
        self.context
            .write_safe()
            .ok()
            .and_then(|mut ctx| ctx.take_folder_contents(username, folder))
    }

    /// Ask `username` where the file we queued with them sits (peer code 51).
    ///
    /// The answer arrives asynchronously and lands on the download, readable
    /// through [`Client::downloads`] as its queue position. A peer we have no
    /// connection to is resolved and the request delivered once it is up, the
    /// same way a browse request is.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection,
    /// or [`crate::SoulseekRs::LockPoisoned`] if the client state is poisoned.
    pub fn request_place_in_queue(
        &self,
        username: &str,
        filename: &str,
    ) -> Result<()> {
        let request = MessageFactory::build_place_in_queue_request(filename);
        self.send_to_peer_or_queue(username, request)
    }

    /// Set our ticker in `room` — the one-line message other members see.
    /// Passing an empty `ticker` clears ours.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn set_room_ticker(&self, room: &str, ticker: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_set_room_ticker(
            room, ticker,
        ))
    }

    /// The ticker board of `room`: what the server sent on join (code 113),
    /// kept current by the later add/remove events.
    #[must_use]
    pub fn room_tickers(&self, room: &str) -> Vec<RoomTicker> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.room_tickers(room),
            Err(e) => {
                error!("[client] room_tickers: {}", e);
                Vec::new()
            }
        }
    }

    /// Subscribe to the global room feed (code 150): every public room message
    /// on the server arrives as a [`crate::RoomEvent::GlobalMessage`].
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn join_global_room(&self) -> Result<()> {
        self.send_server_message(MessageFactory::build_join_global_room())
    }

    /// Stop the global room feed (code 151).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn leave_global_room(&self) -> Result<()> {
        self.send_server_message(MessageFactory::build_leave_global_room())
    }

    /// Add an interest (code 51). Interests are what the recommendation and
    /// similar-user queries below are computed from.
    ///
    /// The server holds interests only for the session that set them, so the
    /// list is kept here and sent again after every login. An interest added
    /// before there is a connection is kept all the same — the error says
    /// only that it could not go out yet, not that it was dropped.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn add_interest(&self, item: &str) -> Result<()> {
        let Some(item) =
            self.context.write_safe()?.add_own_interest(item, true)
        else {
            return Ok(()); // nothing to like or unlike
        };
        self.send_server_message(MessageFactory::build_add_thing_i_like(&item))
    }

    /// Drop an interest (code 52).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn remove_interest(&self, item: &str) -> Result<()> {
        let Some(item) =
            self.context.write_safe()?.remove_own_interest(item, true)
        else {
            return Ok(()); // nothing to like or unlike
        };
        self.send_server_message(MessageFactory::build_remove_thing_i_like(
            &item,
        ))
    }

    /// Add a dislike (code 117).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn add_dislike(&self, item: &str) -> Result<()> {
        let Some(item) =
            self.context.write_safe()?.add_own_interest(item, false)
        else {
            return Ok(()); // nothing to like or unlike
        };
        self.send_server_message(MessageFactory::build_add_thing_i_hate(&item))
    }

    /// Drop a dislike (code 118).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn remove_dislike(&self, item: &str) -> Result<()> {
        let Some(item) =
            self.context.write_safe()?.remove_own_interest(item, false)
        else {
            return Ok(()); // nothing to like or unlike
        };
        self.send_server_message(MessageFactory::build_remove_thing_i_hate(
            &item,
        ))
    }

    /// Ask for recommendations drawn from our own interests (code 54). The
    /// reply arrives asynchronously; read it with
    /// [`Client::recommendations`].
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn request_recommendations(&self) -> Result<()> {
        self.send_server_message(MessageFactory::build_get_recommendations())
    }

    /// Ask for the server-wide recommendations (code 56); read the reply with
    /// [`Client::global_recommendations`].
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn request_global_recommendations(&self) -> Result<()> {
        self.send_server_message(MessageFactory::build_global_recommendations())
    }

    /// The upload speed the server records for us, once it has reported it.
    /// It is what the distributed child limit is derived from, and it only
    /// becomes known after an upload has finished and been reported.
    #[must_use]
    pub fn own_average_speed(&self) -> Option<u32> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.own_average_speed())
    }

    /// What we ourselves like and hate, as set through this client. The
    /// server forgets these when the session ends, so they are re-sent after
    /// each login.
    #[must_use]
    pub fn own_interests(&self) -> UserInterests {
        match self.context.read_safe() {
            Ok(ctx) => ctx.own_interests(),
            Err(e) => {
                error!("[client] own_interests: {}", e);
                UserInterests::default()
            }
        }
    }

    /// The last reply to [`Client::request_recommendations`] as
    /// (recommended, recommended-against), or `None` until one arrives.
    #[must_use]
    pub fn recommendations(
        &self,
    ) -> Option<(Vec<Recommendation>, Vec<Recommendation>)> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.recommendations(false))
    }

    /// The last reply to [`Client::request_global_recommendations`].
    #[must_use]
    pub fn global_recommendations(
        &self,
    ) -> Option<(Vec<Recommendation>, Vec<Recommendation>)> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.recommendations(true))
    }

    /// Ask what else people who like `item` like (code 111).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn request_item_recommendations(&self, item: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_item_recommendations(
            item,
        ))
    }

    /// The last per-item recommendations received for `item`.
    #[must_use]
    pub fn item_recommendations(&self, item: &str) -> Vec<Recommendation> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.item_recommendations(item),
            Err(e) => {
                error!("[client] item_recommendations: {}", e);
                Vec::new()
            }
        }
    }

    /// Ask which users share our interests (code 110).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn request_similar_users(&self) -> Result<()> {
        self.send_server_message(MessageFactory::build_similar_users())
    }

    /// The last similar-user answer, weighted by how much we have in common.
    #[must_use]
    pub fn similar_users(&self) -> Vec<SimilarUser> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.similar_users(),
            Err(e) => {
                error!("[client] similar_users: {}", e);
                Vec::new()
            }
        }
    }

    /// Ask which users like `item` (code 112).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn request_item_similar_users(&self, item: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_item_similar_users(item))
    }

    /// The last answer to [`Client::request_item_similar_users`] for `item`.
    #[must_use]
    pub fn item_similar_users(&self, item: &str) -> Vec<String> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.item_similar_users(item),
            Err(e) => {
                error!("[client] item_similar_users: {}", e);
                Vec::new()
            }
        }
    }

    /// Ask what `username` likes and hates (code 57).
    ///
    /// Any previous answer for `username` is dropped first, so
    /// [`Client::user_interests`] reports the reply to *this* request rather
    /// than a stale one.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn request_user_interests(&self, username: &str) -> Result<()> {
        self.context
            .write_safe()?
            .invalidate_user_interests(username);
        self.send_server_message(MessageFactory::build_user_interests(username))
    }

    /// What the server last told us `username` likes and hates.
    #[must_use]
    pub fn user_interests(&self, username: &str) -> Option<UserInterests> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.user_interests(username))
    }

    /// Give `days` of our own privileges to `username` (code 123).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn give_privileges(&self, username: &str, days: u32) -> Result<()> {
        self.send_server_message(MessageFactory::build_give_privileges(
            username, days,
        ))
    }

    /// Change our account password (code 142).
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn change_password(&self, password: &str) -> Result<()> {
        self.send_server_message(MessageFactory::build_change_password(
            password,
        ))
    }

    /// Send one private message to several users at once (code 149). Each
    /// recipient receives it as an ordinary private message.
    ///
    /// # Errors
    /// [`crate::SoulseekRs::NotConnected`] when there is no server connection.
    pub fn send_private_message_to_many(
        &self,
        usernames: &[String],
        message: &str,
    ) -> Result<()> {
        self.send_server_message(MessageFactory::build_message_users(
            usernames, message,
        ))
    }
}
