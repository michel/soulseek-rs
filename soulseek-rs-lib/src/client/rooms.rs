use super::{
    Client, PeerMessage, Result, RoomEvent, RoomInfo, RwLockExt, ServerMessage,
    SharedDirectory, SoulseekRs, UserMessage, error,
};
use crate::types::{RoomUserStats, UserInfo};

impl Client {
    /// Send a private message to another user via the server.
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn send_private_message(
        &self,
        username: &str,
        message: &str,
    ) -> Result<()> {
        let handle = self
            .server_handle
            .as_ref()
            .ok_or(SoulseekRs::NotConnected)?;
        let msg = crate::message::server::MessageFactory::build_message_user(
            username, message,
        );
        handle
            .send(ServerMessage::SendMessage(msg))
            .map_err(|_| SoulseekRs::NotConnected)?;
        Ok(())
    }

    /// Send a raw server message via the server actor, mapping a dead channel
    /// to [`SoulseekRs::NotConnected`].
    pub(super) fn send_server_message(
        &self,
        message: crate::message::Message,
    ) -> Result<()> {
        self.server_handle
            .as_ref()
            .ok_or(SoulseekRs::NotConnected)?
            .send(ServerMessage::SendMessage(message))
            .map_err(|_| SoulseekRs::NotConnected)?;
        Ok(())
    }

    /// Ask the server for the list of public chat rooms. The response arrives
    /// asynchronously; read it with [`Client::room_list`] or by draining
    /// [`Client::take_room_events`] for a [`RoomEvent::List`].
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn request_room_list(&self) -> Result<()> {
        self.send_server_message(
            crate::message::server::MessageFactory::build_room_list_request(),
        )
    }

    /// Join a public chat room. The membership list and subsequent messages
    /// arrive via [`Client::take_room_events`].
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn join_room(&self, room: &str) -> Result<()> {
        self.send_server_message(
            crate::message::server::MessageFactory::build_join_room(
                room, false,
            ),
        )
    }

    /// Leave a chat room previously joined with [`Client::join_room`].
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn leave_room(&self, room: &str) -> Result<()> {
        self.send_server_message(
            crate::message::server::MessageFactory::build_leave_room(room),
        )
    }

    /// Say `message` in chat room `room`. The server echoes it back as a
    /// [`RoomEvent::Message`], so the UI should render from that echo rather
    /// than optimistically.
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn say_in_room(&self, room: &str, message: &str) -> Result<()> {
        self.send_server_message(
            crate::message::server::MessageFactory::build_say_chatroom(
                room, message,
            ),
        )
    }

    /// The latest snapshot of the public chat-room list.
    #[must_use]
    pub fn room_list(&self) -> Vec<RoomInfo> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.room_list(),
            Err(e) => {
                error!("[client] room_list: {}", e);
                Vec::new()
            }
        }
    }

    /// Ask the server what it knows about `username`: online status and share
    /// statistics.
    ///
    /// Any previous answer for `username` is discarded first, so
    /// [`Client::user_info`] returns `None` until the replies to *this*
    /// request arrive — polling it cannot mistake a stale snapshot for a
    /// fresh one.
    ///
    /// # Errors
    /// Returns [`SoulseekRs::NotConnected`] if the client is not connected.
    pub fn request_user_info(&self, username: &str) -> Result<()> {
        use crate::message::server::MessageFactory;
        self.context.write_safe()?.invalidate_user_info(username);
        self.send_server_message(MessageFactory::build_get_user_status(
            username,
        ))?;
        self.send_server_message(MessageFactory::build_get_user_stats(username))
    }

    /// What the server has told us about `username` since the last
    /// [`Client::request_user_info`], or `None` before any reply arrives.
    ///
    /// Presence and statistics are separate replies, so a snapshot can carry
    /// one and not the other; [`UserInfo::is_complete`] reports when both
    /// have landed.
    #[must_use]
    pub fn user_info(&self, username: &str) -> Option<UserInfo> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.user_info(username),
            Err(e) => {
                error!("[client] user_info: {}", e);
                None
            }
        }
    }

    /// Who is currently in `room`, sorted by name.
    ///
    /// The roster is built from the membership the server sends when
    /// [`Client::join_room`] succeeds and kept current by the join and leave
    /// events after it, so it is populated shortly after joining rather than
    /// immediately. Returns an empty list for a room this client has not
    /// joined.
    #[must_use]
    pub fn room_members(&self, room: &str) -> Vec<String> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.room_members(room),
            Err(e) => {
                error!("[client] room_members: {}", e);
                Vec::new()
            }
        }
    }

    /// What the server reported about the members of `room` when we joined:
    /// their status, share statistics, free upload slots and country.
    ///
    /// Populated from the same reply that fills [`Client::room_members`], so
    /// it lands shortly after joining. Unlike the roster it is not updated by
    /// later join and leave events -- it is the snapshot taken at join time.
    /// Returns an empty list for a room this client has not joined.
    #[must_use]
    pub fn room_member_stats(&self, room: &str) -> Vec<RoomUserStats> {
        match self.context.read_safe() {
            Ok(ctx) => ctx.room_member_stats(room),
            Err(e) => {
                error!("[client] room_member_stats: {}", e);
                Vec::new()
            }
        }
    }

    /// Remove and return all chat-room events received since the last call.
    #[must_use]
    pub fn take_room_events(&self) -> Vec<RoomEvent> {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.take_room_events(),
            Err(e) => {
                error!("[client] take_room_events: {}", e);
                Vec::new()
            }
        }
    }

    /// Request a peer's shared-file listing. When it arrives it can be
    /// retrieved with [`Client::take_browse_result`].
    ///
    /// # Errors
    /// Returns an error if the client's context lock is poisoned.
    pub fn browse_user(&self, username: &str) -> Result<()> {
        let request =
            crate::message::server::MessageFactory::build_get_share_file_list();
        let (connected, registry) = {
            let mut ctx = self.context.write_safe()?;
            ctx.mark_browse_pending(username);
            (
                ctx.peer_registry
                    .as_ref()
                    .is_some_and(|r| r.contains(username)),
                ctx.peer_registry.clone(),
            )
        };
        if connected {
            if let Some(registry) = registry {
                let _ = registry
                    .send_to_peer(username, PeerMessage::SendMessage(request));
            }
        } else {
            self.context
                .write_safe()?
                .queue_peer_message(username, request);
            if let Some(handle) = &self.server_handle {
                let _ = handle
                    .send(ServerMessage::GetPeerAddress(username.to_string()));
            }
        }
        Ok(())
    }

    /// Remove and return a peer's shared-file listing requested via
    /// [`Client::browse_user`], if it has arrived.
    #[must_use]
    pub fn take_browse_result(
        &self,
        username: &str,
    ) -> Option<Vec<SharedDirectory>> {
        self.context
            .write_safe()
            .ok()
            .and_then(|mut ctx| ctx.take_browse_result(username))
    }

    /// Remove and return all private messages received since the last call.
    #[must_use]
    pub fn take_private_messages(&self) -> Vec<UserMessage> {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.take_private_messages(),
            Err(e) => {
                error!("[client] take_private_messages: {}", e);
                Vec::new()
            }
        }
    }
}
