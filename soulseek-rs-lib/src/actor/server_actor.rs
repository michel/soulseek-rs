use crate::actor::{Actor, ActorHandle, ConnectionState};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::server::CheckPrivilegesHandler;
use crate::message::server::ConnectToPeerHandler;
use crate::message::server::ExcludedSearchPhrasesHandler;
use crate::message::server::FileSearchHandler;
use crate::message::server::GetPeerAddressHandler;
use crate::message::server::JoinRoomHandler;
use crate::message::server::LeaveRoomHandler;
use crate::message::server::LoginHandler;
use crate::message::server::MessageFactory;
use crate::message::server::MessageUser;
use crate::message::server::ParentMinSpeedHandler;
use crate::message::server::ParentSpeedRatioHandler;
use crate::message::server::PrivilegedUsersHandler;
use crate::message::server::ReloggedHandler;
use crate::message::server::SayChatroomHandler;
use crate::message::server::UserJoinedRoomHandler;
use crate::message::server::UserLeftRoomHandler;
use crate::message::server::WatchUserHandler;
use crate::message::server::WishListIntervalHandler;
use crate::message::server::{
    GetUserStatsHandler, GetUserStatusHandler, RoomListHandler,
};
use crate::message::{Handlers, MessageType};
use crate::message::{Message, MessageReader};
use crate::peer::ConnectionType;
use crate::peer::Peer;
use crate::types::{
    ClientVersion, RoomEvent, RoomInfo, RoomUserStats, SessionLoss,
    SessionWatch,
};
use crate::utils::lock::RwLockExt;

use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::{SoulseekRs, debug, error, trace, warn};

/// Ceiling on the wait for the server's login verdict. A loaded server can
/// take seconds to answer, so this stays inside the caller's own 45s bound
/// rather than undercutting it.
const LOGIN_VERDICT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct PeerAddress {
    host: String,
    port: u16,
}

impl PeerAddress {
    #[must_use]
    pub const fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    #[must_use]
    pub fn get_host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn get_port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Default)]
pub struct Context {
    pub logged_in: Option<bool>,
}

impl Context {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}
#[derive(Debug, Clone)]
pub struct UserMessage {
    id: u32,
    timestamp: u32,
    username: String,
    message: String,
    new_message: bool,
}
impl UserMessage {
    #[must_use]
    pub const fn new(
        id: u32,
        timestamp: u32,
        username: String,
        message: String,
        new_message: bool,
    ) -> Self {
        Self {
            id,
            timestamp,
            username,
            message,
            new_message,
        }
    }
    /// The server-assigned id of this message (used to acknowledge it).
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Unix timestamp the server recorded for this message.
    #[must_use]
    pub const fn timestamp(&self) -> u32 {
        self.timestamp
    }

    /// The username of the sender.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The message body.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the server flagged this as freshly delivered (as opposed to a
    /// message replayed because it was queued while the recipient was offline).
    #[must_use]
    pub const fn is_new(&self) -> bool {
        self.new_message
    }
}

/// The server actor's mailbox. Marked non-exhaustive: each protocol message
/// the client learns adds a variant, and that must not break callers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ServerMessage {
    ProcessRead,
    LoginStatus(bool),
    /// The server is closing this connection: the same username logged in
    /// elsewhere.
    Relogged,
    SendMessage(Message),
    Login {
        username: String,
        password: String,
        version: ClientVersion,
        response: std::sync::mpsc::Sender<Result<bool, SoulseekRs>>,
    },
    FileSearch {
        token: u32,
        query: String,
    },
    /// A wishlist search (code 103): a stored query the server lets us repeat
    /// once per announced interval.
    WishlistSearch {
        token: u32,
        query: String,
    },
    /// The server announced how often it will accept a wishlist search.
    WishlistInterval(u32),
    /// Everyone the server counts as privileged (code 69).
    PrivilegedUsers(Vec<String>),
    /// Seconds of our own privileges left (code 92).
    OwnPrivileges(u32),
    /// Ask the server for the answer to the above.
    CheckPrivileges,
    /// A search the server distributed to us from another user; if it matches
    /// our shares we reply with a FileSearchResponse.
    FileSearchRequest {
        username: String,
        token: u32,
        query: String,
    },
    #[allow(dead_code)]
    ConnectToPeer(Peer),
    PierceFirewall(u32),
    GetPeerAddress(String),
    GetPeerAddressResponse {
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    },
    PrivateMessageReceived(UserMessage),
    RoomListReceived(Vec<RoomInfo>),
    /// A `GetUserStatus` (code 7) reply.
    UserStatusReceived {
        username: String,
        status: u32,
        privileged: bool,
    },
    /// A `WatchUser` (code 5) reply: the initial snapshot for a user we just
    /// started watching. The stats are absent when the server does not know
    /// the username.
    WatchedUserReceived {
        username: String,
        exists: bool,
        status: Option<u32>,
        average_speed: Option<u32>,
        shared_files: Option<u32>,
        shared_folders: Option<u32>,
    },
    /// A `GetUserStats` (code 36) reply.
    UserStatsReceived {
        username: String,
        average_speed: u32,
        shared_files: u32,
        shared_folders: u32,
    },
    RoomJoined {
        room: String,
        users: Vec<String>,
    },
    /// Per-member statistics carried by the same `JoinRoom` (code 14) reply.
    RoomMemberStats {
        room: String,
        stats: Vec<RoomUserStats>,
    },
    RoomLeft {
        room: String,
    },
    RoomMessageReceived {
        room: String,
        username: String,
        message: String,
    },
    RoomUserJoined {
        room: String,
        username: String,
    },
    RoomUserLeft {
        room: String,
        username: String,
    },
}

pub struct ServerActor {
    address: PeerAddress,
    context: Arc<RwLock<Context>>,
    listen_port: u16,
    enable_listen: bool,
    stream: Option<TcpStream>,
    connection_state: ConnectionState,
    reader: MessageReader,
    client_channel: Sender<ClientOperation>,
    self_handle: Option<ActorHandle<ServerMessage>>,
    dispatcher: Option<MessageDispatcher<ServerMessage>>,
    dispatcher_receiver: Option<Receiver<ServerMessage>>,
    dispatcher_sender: Option<Sender<ServerMessage>>,
    queued_messages: Vec<ServerMessage>,
    shared_folder_count: u32,
    shared_file_count: u32,
    session: SessionWatch,
}

/// The messages a client sends right after a successful login: its shared-file
/// counts, distributed-network opt-out, online status, and (when listening) the
/// port peers should connect to. Kept as a free function so it can be tested
/// without a live connection.
fn post_login_messages(
    enable_listen: bool,
    listen_port: u16,
    shared_folders: u32,
    shared_files: u32,
) -> Vec<Message> {
    let mut messages = vec![
        MessageFactory::build_shared_folders_message(
            shared_folders,
            shared_files,
        ),
        MessageFactory::build_no_parent_message(),
        MessageFactory::build_set_status_message(2),
    ];
    if enable_listen {
        messages.push(MessageFactory::build_set_wait_port_message(listen_port));
    }
    messages
}

impl ServerActor {
    #[must_use]
    pub fn new(
        address: PeerAddress,
        client_channel: Sender<ClientOperation>,
        listen_port: u16,
        enable_listen: bool,
        shared_folder_count: u32,
        shared_file_count: u32,
    ) -> Self {
        Self {
            address,
            context: Arc::new(RwLock::new(Context::new())),
            listen_port,
            enable_listen,
            stream: None,
            connection_state: ConnectionState::Disconnected,
            dispatcher: None,
            dispatcher_receiver: None,
            dispatcher_sender: None,
            reader: MessageReader::new(),
            client_channel,
            self_handle: None,
            queued_messages: Vec::new(),
            shared_folder_count,
            shared_file_count,
            session: SessionWatch::default(),
        }
    }

    /// Share the client's view of whether this session is still alive.
    pub fn set_session_watch(&mut self, session: SessionWatch) {
        self.session = session;
    }

    fn initiate_connection(&mut self) {
        let stream = match TcpStream::connect((
            self.address.host.as_str(),
            self.address.port,
        )) {
            Ok(s) => s,
            Err(e) => {
                error!("[server] Failed to connect to {}: {}", self.address, e);
                self.disconnect_with_error();
                return;
            }
        };

        if let Err(e) = stream.set_nonblocking(true) {
            error!("[server] Failed to set non-blocking: {}", e);
            self.disconnect_with_error();
            return;
        }
        stream.set_nodelay(true).ok();

        self.stream = Some(stream);
        self.connection_state = ConnectionState::Connecting {
            since: Instant::now(),
        };
    }

    pub fn set_self_handle(&mut self, handle: ActorHandle<ServerMessage>) {
        self.self_handle = Some(handle);
    }

    fn initialize_dispatcher(&mut self) {
        let (dispatcher_sender, dispatcher_receiver) =
            std::sync::mpsc::channel::<ServerMessage>();

        self.dispatcher_receiver = Some(dispatcher_receiver);
        self.dispatcher_sender = Some(dispatcher_sender.clone());

        if let Err(e) = self
            .client_channel
            .send(ClientOperation::SetServerSender(dispatcher_sender.clone()))
        {
            error!("[server] failed to send SetServerSender: {}", e);
        }

        let mut handlers = Handlers::new();

        handlers.register_handler(LoginHandler);
        handlers.register_handler(ReloggedHandler);
        handlers.register_handler(RoomListHandler);
        handlers.register_handler(GetUserStatusHandler);
        handlers.register_handler(WatchUserHandler);
        handlers.register_handler(GetUserStatsHandler);
        handlers.register_handler(JoinRoomHandler);
        handlers.register_handler(LeaveRoomHandler);
        handlers.register_handler(SayChatroomHandler);
        handlers.register_handler(UserJoinedRoomHandler);
        handlers.register_handler(UserLeftRoomHandler);
        handlers.register_handler(ExcludedSearchPhrasesHandler);
        handlers.register_handler(PrivilegedUsersHandler);
        handlers.register_handler(MessageUser);
        handlers.register_handler(WishListIntervalHandler);
        handlers.register_handler(ParentMinSpeedHandler);
        handlers.register_handler(ParentSpeedRatioHandler);
        handlers.register_handler(CheckPrivilegesHandler);
        handlers.register_handler(FileSearchHandler);
        handlers.register_handler(GetPeerAddressHandler);
        handlers.register_handler(ConnectToPeerHandler);

        self.dispatcher = Some(MessageDispatcher::new(
            "server".into(),
            dispatcher_sender,
            handlers,
        ));
    }

    fn process_dispatcher_messages(&mut self) {
        let messages: Vec<ServerMessage> = self
            .dispatcher_receiver
            .as_ref()
            .map_or_else(Vec::new, |receiver| receiver.try_iter().collect());

        for msg in &messages {
            self.handle_message(msg.clone());
        }
    }

    pub fn file_search(&mut self, token: u32, query: &str) {
        self.queue_message(MessageFactory::build_file_search_message(
            token, query,
        ));
    }

    fn handle_message(&mut self, msg: ServerMessage) {
        if !matches!(self.connection_state, ConnectionState::Connected) {
            if matches!(&msg, ServerMessage::ProcessRead) {
                // Always process read operations
            } else {
                // Queue all other messages when not connected
                self.queued_messages.push(msg);
                return;
            }
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
            other => {
                error!("[server] unroutable message: {:?}", other);
            }
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

    fn handle_login_status(&mut self, message: bool) {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.logged_in = Some(message),
            Err(e) => {
                error!("[server] LoginStatus write: {}", e);
            }
        }
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

    fn handle_login(
        &mut self,
        username: String,
        password: String,
        version: ClientVersion,
        response: std::sync::mpsc::Sender<Result<bool, SoulseekRs>>,
    ) {
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
                username,
                token,
                query,
            })
        {
            error!("[server] forward IncomingSearch: {}", e);
        }
    }

    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages();
        }

        {
            let Some(stream) = self.stream.as_mut() else {
                return;
            };

            match self.reader.read_from_socket(stream) {
                Ok(()) => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    debug!("[server] Read operation timed out",);
                }
                Err(e) => {
                    error!(
                        "[server] Error reading from server: {} (kind: {:?}). Disconnecting.",
                        e,
                        e.kind()
                    );
                    self.disconnect_with_error();
                    return;
                }
            }
        }
        self.extract_and_process_messages();
    }

    fn extract_and_process_messages(&mut self) {
        let mut extracted_count = 0;
        loop {
            match self.reader.extract_message() {
                Ok(Some(mut message)) => {
                    extracted_count += 1;
                    trace!(
                        "[server] ← Message #{}: {:?}",
                        extracted_count,
                        message
                            .get_message_name(
                                MessageType::Server,
                                u32::from(message.get_message_code())
                            )
                            .map_err(|e| e.to_string())
                    );
                    if let Some(ref dispatcher) = self.dispatcher {
                        dispatcher.dispatch(&mut message);
                    } else {
                        warn!("[server] No dispatcher available!",);
                    }
                }
                Err(e) => {
                    warn!(
                        "[server] Error extracting message: {}. Disconnecting.",
                        e
                    );
                    self.disconnect_with_error();
                    return;
                }
                Ok(None) => {
                    break;
                }
            }
        }

        self.process_dispatcher_messages();
    }

    /// Forward a chat-room event to the client operations loop.
    fn forward_room_event(&self, event: RoomEvent) {
        if let Err(e) =
            self.client_channel.send(ClientOperation::RoomEvent(event))
        {
            error!("[server] Error forwarding room event to client: {}", e);
        }
    }

    fn queue_message(&mut self, message: Message) {
        if let Some(sender) = &self.dispatcher_sender {
            match sender.send(ServerMessage::SendMessage(message)) {
                Ok(()) => {}
                Err(e) => error!("Failed to send: {}", e),
            }
        } else {
            self.queued_messages
                .push(ServerMessage::SendMessage(message));
        }
    }

    fn send_message(&mut self, message: Message) {
        let Some(stream) = self.stream.as_mut() else {
            error!("[server] Cannot send message: stream is None");
            return;
        };

        trace!(
            "[server] ➡ {:?}",
            message
                .get_message_name(
                    MessageType::Server,
                    u32::from_le_bytes(
                        message.get_slice(0, 4).try_into().unwrap_or_default()
                    )
                )
                .map_err(|e| e.to_string())
        );

        if let Err(e) = stream.write_all(&message.get_buffer()) {
            error!("[server] Error writing message: {}. Disconnecting.", e);
            self.disconnect_with_error();
            return;
        }

        if let Err(e) = stream.flush() {
            error!("[server] Error flushing stream: {}. Disconnecting.", e);
            self.disconnect_with_error();
        }
    }

    /// The server is closing this connection because the same account logged
    /// in elsewhere. Nothing reconnects it, so the session is over.
    fn handle_relogged(&mut self) {
        error!(
            "[server] another login took this username; this session has been \
             closed by the server"
        );
        self.session.record(SessionLoss::Displaced);
        self.disconnect();
    }

    fn disconnect_with_error(&mut self) {
        debug!("[server] disconnect");

        // Losing an established connection ends the session: nothing reconnects
        // it, so anything still waiting on the network is waiting for good.
        if matches!(self.connection_state, ConnectionState::Connected) {
            self.session.record(SessionLoss::Disconnected);
        }
        self.connection_state = ConnectionState::Disconnected;
        self.stream.take();
    }

    fn disconnect(&mut self) {
        debug!("[server] disconnected");

        self.stream.take();
        self.connection_state = ConnectionState::Disconnected;
    }

    fn check_connection_status(&mut self) {
        let ConnectionState::Connecting { since } = self.connection_state
        else {
            return;
        };

        if since.elapsed() > Duration::from_secs(20) {
            error!("[server] Connection timeout after 20 seconds");
            self.disconnect_with_error();
            return;
        }

        let Some(ref stream) = self.stream else {
            return;
        };

        match stream.peer_addr() {
            Ok(_) => {
                self.connection_state = ConnectionState::Connected;
                self.on_connection_established();
            }
            Err(ref e) if e.kind() == io::ErrorKind::NotConnected => {}
            Err(e) => {
                error!("[server] Connection failed: {}", e);
                self.disconnect_with_error();
            }
        }
    }

    fn on_connection_established(&mut self) {
        self.initialize_dispatcher();

        let queued = std::mem::take(&mut self.queued_messages);
        for msg in queued {
            self.handle_message(msg);
        }

        if let Some(ref handle) = self.self_handle {
            handle.send(ServerMessage::ProcessRead).ok();
        }

        self.process_read();
    }
}

impl Actor for ServerActor {
    type Message = ServerMessage;

    fn handle(&mut self, msg: Self::Message) {
        self.handle_message(msg);
    }

    fn on_start(&mut self) {
        if self.stream.is_none() {
            self.initiate_connection();
        } else {
            self.connection_state = ConnectionState::Connected;
            self.on_connection_established();
        }
    }

    fn on_stop(&mut self) {
        trace!("[server] actor stopping");
        self.disconnect();
    }

    fn tick(&mut self) {
        match self.connection_state {
            ConnectionState::Connecting { .. } => {
                self.check_connection_status();
            }
            ConnectionState::Connected => {
                if self.stream.is_some() {
                    self.process_read();
                }
            }
            ConnectionState::Disconnected => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(message: &Message) -> u32 {
        u32::from_le_bytes(message.get_data()[0..4].try_into().unwrap())
    }

    #[test]
    fn a_timed_out_connect_parks_the_actor_in_disconnected() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut actor = ServerActor::new(
            PeerAddress::new("127.0.0.1".to_string(), 1),
            tx,
            0,
            false,
            0,
            0,
        );
        actor.connection_state = ConnectionState::Connecting {
            since: Instant::now().checked_sub(Duration::from_secs(21)).unwrap(),
        };

        actor.tick();

        assert!(
            matches!(actor.connection_state, ConnectionState::Disconnected),
            "a timed-out connect must leave Connecting"
        );
    }

    #[test]
    fn post_login_messages_carry_counts_and_conditional_wait_port() {
        let messages = post_login_messages(true, 4321, 3, 7);
        let codes: Vec<u32> = messages.iter().map(code_of).collect();
        // SharedFolders, HaveNoParent, SetStatus, SetWaitPort.
        assert_eq!(codes, vec![35, 71, 28, 2]);

        // The SharedFolders message (code 35) carries the real counts.
        let shared = messages[0].get_data();
        assert_eq!(u32::from_le_bytes(shared[4..8].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(shared[8..12].try_into().unwrap()), 7);

        // Not listening omits SetWaitPort (code 2).
        let no_listen = post_login_messages(false, 4321, 3, 7);
        let codes: Vec<u32> = no_listen.iter().map(code_of).collect();
        assert_eq!(codes, vec![35, 71, 28]);
    }
}
