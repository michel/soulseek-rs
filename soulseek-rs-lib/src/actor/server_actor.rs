use crate::actor::{Actor, ActorHandle, ConnectionState};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
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
use crate::message::server::RoomListHandler;
use crate::message::server::SayChatroomHandler;
use crate::message::server::UserJoinedRoomHandler;
use crate::message::server::UserLeftRoomHandler;
use crate::message::server::WishListIntervalHandler;
use crate::message::{Handlers, MessageType};
use crate::message::{Message, MessageReader};
use crate::peer::ConnectionType;
use crate::peer::Peer;
use crate::types::{RoomEvent, RoomInfo};
use crate::utils::lock::RwLockExt;

use std::io::{self, Error, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::{SoulseekRs, debug, error, trace, warn};

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
    pub fn print(&self) {
        debug!(
            "Timestamp: {}. User: {}, Id: #{}, New message: {} Message: {}",
            self.timestamp,
            self.username,
            self.id,
            self.new_message,
            self.message
        );
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

#[derive(Debug, Clone)]
pub enum ServerMessage {
    ProcessRead,
    LoginStatus(bool),
    SendMessage(Message),
    Login {
        username: String,
        password: String,
        response: std::sync::mpsc::Sender<Result<bool, SoulseekRs>>,
    },
    FileSearch {
        token: u32,
        query: String,
    },
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
    RoomJoined {
        room: String,
        users: Vec<String>,
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
        }
    }

    #[must_use]
    pub const fn get_address(&self) -> &PeerAddress {
        &self.address
    }

    #[must_use]
    pub const fn get_sender(&self) -> Option<&Sender<ServerMessage>> {
        self.dispatcher_sender.as_ref()
    }

    fn initiate_connection(&mut self) -> bool {
        let host = self.address.host.clone();
        let port = self.address.port;

        let addr_str = format!("{host}:{port}");

        let mut socket_addrs = match addr_str.to_socket_addrs() {
            Ok(addrs) => addrs,
            Err(e) => {
                error!("[server] Failed to resolve address: {}", e);

                self.disconnect_with_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    e,
                ));
                return false;
            }
        };

        let socket_addr = socket_addrs.next();

        let Some(addr) = socket_addr else {
            let error_msg =
                format!("No socket addresses found for {host}:{port}");
            error!("[server] {}", error_msg);
            self.disconnect_with_error(io::Error::new(
                io::ErrorKind::InvalidInput,
                error_msg,
            ));
            return false;
        };

        let stream = match TcpStream::connect(addr) {
            Ok(s) => s,
            Err(e) => {
                self.disconnect_with_error(e);
                return false;
            }
        };

        if let Err(e) = stream.set_nonblocking(true) {
            error!("[server] Failed to set non-blocking: {}", e);
            self.disconnect_with_error(e);
            return false;
        }
        stream.set_nodelay(true).ok();

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
        handlers.register_handler(RoomListHandler);
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
        handlers.register_handler(PrivilegedUsersHandler);
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
            .map_or_else(Vec::new, |receiver| {
                let mut msgs = Vec::new();
                while let Ok(msg) = receiver.try_recv() {
                    msgs.push(msg);
                }
                msgs
            });

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
            ServerMessage::LoginStatus(message) => {
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
                debug!(
                    "[server] Received GetPeerAddress response for {}: {}:{} (obf_type: {}, obf_port: {})",
                    username, host, port, obfuscation_type, obfuscated_port
                );

                if let Err(e) = self.client_channel.send(
                    ClientOperation::GetPeerAddressResponse {
                        username,
                        host,
                        port,
                        obfuscation_type,
                        obfuscated_port,
                    },
                ) {
                    error!(
                        "[server] Error forwarding GetPeerAddress response to client: {}",
                        e
                    );
                }
            }
            ServerMessage::PrivateMessageReceived(user_message) => {
                debug!(
                    "[server] Private message from {}",
                    user_message.username()
                );
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
            ServerMessage::RoomListReceived(rooms) => {
                self.forward_room_event(RoomEvent::List(rooms));
            }
            ServerMessage::RoomJoined { room, users } => {
                self.forward_room_event(RoomEvent::Joined { room, users });
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
                response,
            } => {
                self.queue_message(MessageFactory::build_login_message(
                    &username, &password,
                ));

                let start = std::time::Instant::now();
                let timeout = Duration::from_secs(5);

                let context = self.context.clone();
                std::thread::spawn(move || {
                    loop {
                        if start.elapsed() >= timeout {
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
            ServerMessage::FileSearch { token, query } => {
                self.file_search(token, &query);
            }
            ServerMessage::FileSearchRequest {
                username,
                token,
                query,
            } => {
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
                    self.disconnect_with_error(e);
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
                    self.disconnect_with_error(e);
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
                        message.get_slice(0, 4).try_into().unwrap()
                    )
                )
                .map_err(|e| e.to_string())
        );

        if let Err(e) = stream.write_all(&message.get_buffer()) {
            error!("[server] Error writing message: {}. Disconnecting.", e);
            self.disconnect_with_error(e);
            return;
        }

        if let Err(e) = stream.flush() {
            error!("[server] Error flushing stream: {}. Disconnecting.", e);
            self.disconnect_with_error(e);
        }
    }

    fn disconnect_with_error(&mut self, _error: Error) {
        debug!("[server] disconnect");

        self.stream.take();
    }

    fn disconnect(&mut self) {
        debug!("[server] disconnected");

        self.stream.take();
    }

    fn check_connection_status(&mut self) {
        let ConnectionState::Connecting { since } = self.connection_state
        else {
            return;
        };

        if since.elapsed() > Duration::from_secs(20) {
            error!("[server] Connection timeout after 20 seconds");
            self.disconnect_with_error(io::Error::new(
                io::ErrorKind::TimedOut,
                "Connection timeout",
            ));
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
                self.disconnect_with_error(e);
            }
        }
    }

    fn on_connection_established(&mut self) {
        let Some(_) = self.stream else {
            panic!("Stream should be available here")
        };

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
    use super::post_login_messages;
    use crate::message::Message;

    fn code_of(message: &Message) -> u32 {
        u32::from_le_bytes(message.get_data()[0..4].try_into().unwrap())
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
