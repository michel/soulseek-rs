use crate::actor::{Actor, ActorHandle, ConnectionState};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::server::AdminMessageHandler;
use crate::message::server::CantConnectToPeerHandler;
use crate::message::server::CheckPrivilegesHandler;
use crate::message::server::ConnectToPeerHandler;
use crate::message::server::EmbeddedMessageHandler;
use crate::message::server::ExcludedSearchPhrasesHandler;
use crate::message::server::FileSearchHandler;
use crate::message::server::GetPeerAddressHandler;
use crate::message::server::GlobalRoomMessageHandler;
use crate::message::server::JoinRoomHandler;
use crate::message::server::LeaveRoomHandler;
use crate::message::server::LoginHandler;
use crate::message::server::MessageFactory;
use crate::message::server::MessageUser;
use crate::message::server::ParentMinSpeedHandler;
use crate::message::server::ParentSpeedRatioHandler;
use crate::message::server::PossibleParentsHandler;
use crate::message::server::PrivilegedUsersHandler;
use crate::message::server::ReloggedHandler;
use crate::message::server::ResetDistributedHandler;
use crate::message::server::SayChatroomHandler;
use crate::message::server::UserJoinedRoomHandler;
use crate::message::server::UserLeftRoomHandler;
use crate::message::server::WatchUserHandler;
use crate::message::server::WishListIntervalHandler;
use crate::message::server::{
    CantCreateRoomHandler, OwnRoomStandingHandler, RoomMembersHandler,
    RoomOperatorsHandler, RoomRosterChangeHandler,
};
use crate::message::server::{
    GetUserStatsHandler, GetUserStatusHandler, RoomListHandler,
};
use crate::message::server::{
    GlobalRecommendationsHandler, ItemRecommendationsHandler,
    ItemSimilarUsersHandler, RecommendationsHandler, SimilarUsersHandler,
    UserInterestsHandler,
};
use crate::message::server::{
    RoomTickerAddedHandler, RoomTickerRemovedHandler, RoomTickersHandler,
};
use crate::message::{Handlers, MessageType};
use crate::message::{Message, MessageReader};
use crate::peer::ConnectionType;
use crate::peer::Peer;
use crate::types::{
    ClientVersion, Recommendation, RoomEvent, RoomInfo, RoomUserStats,
    SessionLoss, SessionWatch, SimilarUser, UserInterests,
};
use crate::utils::lock::RwLockExt;

use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::{SoulseekRs, debug, error, trace, warn};

mod handlers;
mod types;
pub use types::{Context, PeerAddress, UserMessage};

/// Ceiling on the wait for the server's login verdict. A loaded server can
/// take seconds to answer, so this stays inside the caller's own 45s bound
/// rather than undercutting it.
const LOGIN_VERDICT_TIMEOUT: Duration = Duration::from_secs(30);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// Peers the server suggests as distributed-network parents: username,
    /// host, port.
    PossibleParents(Vec<(String, String, u16)>),
    /// The server asks us to drop our parent and start over.
    ResetDistributed,
    /// The ticker board of a room we just joined (code 113).
    RoomTickers {
        room: String,
        tickers: Vec<crate::types::RoomTicker>,
    },
    /// One member set their ticker (code 114).
    RoomTickerAdded {
        room: String,
        username: String,
        ticker: String,
    },
    /// One member cleared their ticker (code 115).
    RoomTickerRemoved {
        room: String,
        username: String,
    },
    /// A message from the global room feed (code 152).
    GlobalRoomMessageReceived {
        room: String,
        username: String,
        message: String,
    },
    /// Recommendations from our own interests (code 54).
    RecommendationsReceived {
        recommended: Vec<Recommendation>,
        unrecommended: Vec<Recommendation>,
    },
    /// Server-wide recommendations (code 56).
    GlobalRecommendationsReceived {
        recommended: Vec<Recommendation>,
        unrecommended: Vec<Recommendation>,
    },
    /// Recommendations for one item (code 111).
    ItemRecommendationsReceived {
        item: String,
        recommendations: Vec<Recommendation>,
    },
    /// Users similar to us (code 110).
    SimilarUsersReceived {
        users: Vec<SimilarUser>,
    },
    /// Users who like one item (code 112).
    ItemSimilarUsersReceived {
        item: String,
        usernames: Vec<String>,
    },
    /// What another user likes and hates (code 57).
    UserInterestsReceived(UserInterests),
    /// A peer could not connect to us after we asked the server to broker
    /// (code 1001); the token is the one we quoted.
    CantConnectToPeer {
        token: u32,
    },
    /// Phrases the server refuses to search for (code 160).
    ExcludedSearchPhrases(Vec<String>),
    /// The upload speed a client needs before the server lets it carry
    /// children (code 83).
    ParentMinSpeed(u32),
    /// The divisor turning that speed into a child count (code 84).
    ParentSpeedRatio(u32),
    /// Who may enter a private room (code 133).
    PrivateRoomMembers {
        room: String,
        users: Vec<String>,
    },
    /// Who runs a private room (code 148).
    PrivateRoomOperators {
        room: String,
        users: Vec<String>,
    },
    /// One user joined or left a private room's member or operator roster
    /// (codes 134/135 and 143/144).
    PrivateRoomRosterChanged {
        room: String,
        username: String,
        /// True for the member roster, false for the operator roster.
        members: bool,
        added: bool,
    },
    /// Our own membership (139/140) or operatorship (145/146) of a private
    /// room was granted or revoked.
    OwnRoomStandingChanged {
        room: String,
        members: bool,
        granted: bool,
    },
    /// The room we asked to join could not be created (code 1003).
    CantCreateRoom {
        room: String,
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
/// counts, online status, and (when listening)
/// the port peers should connect to. Kept as a free function so it can be
/// tested without a live connection.
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

    fn initiate_connection(&mut self) -> bool {
        let stream = (self.address.get_host(), self.address.get_port())
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| {
                addrs.find_map(|addr| {
                    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()
                })
            });
        let Some(stream) = stream else {
            error!("[server] Failed to connect to {}", self.address);
            self.disconnect_with_error();
            return false;
        };

        if let Err(e) = stream.set_nonblocking(true) {
            error!("[server] Failed to set non-blocking: {}", e);
            self.disconnect_with_error();
            return false;
        }
        stream.set_nodelay(true).ok();
        if let Err(e) = crate::utils::keepalive::set_keepalive(&stream) {
            warn!("[server] keepalive not set: {}", e);
        }

        self.stream = Some(stream);
        self.connection_state = ConnectionState::Connecting {
            since: Instant::now(),
        };
        true
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
        handlers.register_handler(AdminMessageHandler);
        handlers.register_handler(PossibleParentsHandler);
        handlers.register_handler(ResetDistributedHandler);
        handlers.register_handler(EmbeddedMessageHandler);
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
        handlers.register_handler(CantConnectToPeerHandler);
        handlers.register_handler(RoomMembersHandler);
        handlers.register_handler(RoomOperatorsHandler);
        // Members added/removed (134/135), operators added/removed (143/144).
        for code in [134, 135, 143, 144] {
            handlers.register_handler(RoomRosterChangeHandler(code));
        }
        // Our own membership (139/140) and operatorship (145/146).
        for code in [139, 140, 145, 146] {
            handlers.register_handler(OwnRoomStandingHandler(code));
        }
        handlers.register_handler(CantCreateRoomHandler);
        handlers.register_handler(RoomTickersHandler);
        handlers.register_handler(RoomTickerAddedHandler);
        handlers.register_handler(RoomTickerRemovedHandler);
        handlers.register_handler(GlobalRoomMessageHandler);
        handlers.register_handler(RecommendationsHandler);
        handlers.register_handler(GlobalRecommendationsHandler);
        handlers.register_handler(ItemRecommendationsHandler);
        handlers.register_handler(SimilarUsersHandler);
        handlers.register_handler(ItemSimilarUsersHandler);
        handlers.register_handler(UserInterestsHandler);

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
                                message.get_message_code()
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

        if matches!(self.connection_state, ConnectionState::Connected) {
            self.session.record(SessionLoss::Disconnected);
        }
        self.disconnect();
    }

    fn disconnect(&mut self) {
        debug!("[server] disconnected");

        self.stream.take();
        self.connection_state = ConnectionState::Disconnected;
        self.dispatcher = None;
        self.dispatcher_sender = None;
        self.dispatcher_receiver = None;
        self.queued_messages.clear();
        self.reader = MessageReader::new();
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.logged_in = None;
        }
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
            let _ = self.initiate_connection();
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
mod tests;
