use crate::actor::{Actor, ActorHandle, ConnectionState, OutboundBuffer};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::server::ConnectToPeerHandler;
use crate::message::server::ExcludedSearchPhrasesHandler;
use crate::message::server::FileSearchHandler;
use crate::message::server::GetPeerAddressHandler;
use crate::message::server::LoginHandler;
use crate::message::server::MessageFactory;
use crate::message::server::MessageUser;
use crate::message::server::ParentMinSpeedHandler;
use crate::message::server::ParentSpeedRatioHandler;
use crate::message::server::PrivilegedUsersHandler;
use crate::message::server::RoomListHandler;
use crate::message::server::WishListIntervalHandler;
use crate::message::{Handlers, MessageType};
use crate::message::{Message, MessageReader};
use crate::peer::ConnectionType;
use crate::peer::Peer;

use mio::event::Event;
use mio::net::TcpStream;
use mio::{Interest, Registry, Token};
use std::io::{self, Error};
use std::net::ToSocketAddrs;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::{SoulseekRs, debug, error, info, trace, warn};

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

    /// Whether the server flagged this as freshly delivered.
    #[must_use]
    pub const fn is_new(&self) -> bool {
        self.new_message
    }
}

#[derive(Debug, Clone)]
pub enum ServerMessage {
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
}

struct PendingLogin {
    username: String,
    response: Sender<Result<bool, SoulseekRs>>,
    deadline: Instant,
}

pub struct ServerActor {
    address: PeerAddress,
    context: Context,
    listen_port: u16,
    enable_listen: bool,
    stream: Option<TcpStream>,
    connection_state: ConnectionState,
    reader: MessageReader,
    client_channel: ActorHandle<ClientOperation>,
    self_handle: Option<ActorHandle<ServerMessage>>,
    dispatcher: Option<MessageDispatcher<ServerMessage>>,
    pending_login: Option<PendingLogin>,
    queued_messages: Vec<ServerMessage>,
    outbound: OutboundBuffer,
    io_generation: u64,
}

impl ServerActor {
    #[must_use]
    pub fn new(
        address: PeerAddress,
        client_channel: ActorHandle<ClientOperation>,
        listen_port: u16,
        enable_listen: bool,
    ) -> Self {
        Self {
            address,
            context: Context::new(),
            listen_port,
            enable_listen,
            stream: None,
            connection_state: ConnectionState::Disconnected,
            dispatcher: None,
            pending_login: None,
            reader: MessageReader::new(),
            client_channel,
            self_handle: None,
            queued_messages: Vec::new(),
            outbound: OutboundBuffer::new(),
            io_generation: 0,
        }
    }

    #[must_use]
    pub const fn get_address(&self) -> &PeerAddress {
        &self.address
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

        stream.set_nodelay(true).ok();

        self.stream = Some(stream);
        self.connection_state = ConnectionState::Connecting {
            since: Instant::now(),
        };
        self.bump_io_generation();
        true
    }

    pub fn set_self_handle(&mut self, handle: ActorHandle<ServerMessage>) {
        self.self_handle = Some(handle);
    }

    fn initialize_dispatcher(&mut self) {
        if let Some(handle) = self.self_handle.clone() {
            if let Err(e) = self
                .client_channel
                .send(ClientOperation::SetServerHandle(handle))
            {
                error!("[server] failed to send SetServerHandle: {}", e);
            }
        } else {
            error!("[server] self handle unavailable during dispatcher init");
        }

        let mut handlers = Handlers::new();

        handlers.register_handler(LoginHandler);
        handlers.register_handler(RoomListHandler);
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

        self.dispatcher =
            Some(MessageDispatcher::new("server".into(), handlers));
    }

    pub fn file_search(&mut self, token: u32, query: &str) {
        self.queue_message(MessageFactory::build_file_search_message(
            token, query,
        ));
    }

    fn start_login(
        &mut self,
        username: String,
        password: String,
        response: Sender<Result<bool, SoulseekRs>>,
    ) {
        if let Some(pending) = self.pending_login.replace(PendingLogin {
            username: username.clone(),
            response,
            deadline: Instant::now() + Duration::from_secs(5),
        }) {
            let _ = pending.response.send(Err(SoulseekRs::Timeout));
        }

        self.context.logged_in = None;
        self.queue_message(MessageFactory::build_login_message(
            &username, &password,
        ));
    }

    fn complete_login(&mut self, logged_in: bool) {
        self.context.logged_in = Some(logged_in);

        let Some(pending) = self.pending_login.take() else {
            return;
        };

        if logged_in {
            info!("Logged in as {}", pending.username);
            let _ = pending.response.send(Ok(true));
            self.queue_message(MessageFactory::build_shared_folders_message(
                1, 499,
            ));
            self.queue_message(MessageFactory::build_no_parent_message());
            self.queue_message(MessageFactory::build_set_status_message(2));
            if self.enable_listen {
                self.queue_message(
                    MessageFactory::build_set_wait_port_message(
                        self.listen_port,
                    ),
                );
            }
        } else {
            let _ =
                pending.response.send(Err(SoulseekRs::AuthenticationFailed));
        }
    }

    fn check_login_timeout(&mut self) {
        let Some(pending) = self.pending_login.as_ref() else {
            return;
        };

        if Instant::now() < pending.deadline {
            return;
        }

        warn!("Timeout waiting for login response");
        if let Some(pending) = self.pending_login.take() {
            let _ = pending.response.send(Err(SoulseekRs::Timeout));
        }
    }

    fn handle_message(&mut self, msg: ServerMessage) {
        if !matches!(self.connection_state, ConnectionState::Connected) {
            match msg {
                ServerMessage::Login {
                    username,
                    password,
                    response,
                } => {
                    self.start_login(username, password, response);
                }
                other => {
                    self.queued_messages.push(other);
                }
            }
            return;
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
                self.complete_login(message);
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
            ServerMessage::Login {
                username,
                password,
                response,
            } => {
                self.start_login(username, password, response);
            }
            ServerMessage::FileSearch { token, query } => {
                self.file_search(token, &query);
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
                    let messages = if let Some(ref dispatcher) = self.dispatcher
                    {
                        dispatcher.dispatch(&mut message)
                    } else {
                        warn!("[server] No dispatcher available!",);
                        Vec::new()
                    };
                    for msg in messages {
                        self.handle_message(msg);
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
    }

    fn queue_message(&mut self, message: Message) {
        if matches!(self.connection_state, ConnectionState::Connected) {
            self.send_message(message);
        } else {
            self.queued_messages
                .push(ServerMessage::SendMessage(message));
        }
    }

    fn send_message(&mut self, message: Message) {
        if self.stream.is_none() {
            error!("[server] Cannot send message: stream is None");
            return;
        }

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

        self.outbound.push(message.get_buffer());
        self.bump_io_generation();
        self.flush_outbound();
    }

    fn flush_outbound(&mut self) {
        let was_empty = self.outbound.is_empty();
        let Some(stream) = self.stream.as_mut() else {
            return;
        };

        if let Err(e) = self.outbound.flush(stream) {
            error!("[server] Error writing message: {}. Disconnecting.", e);
            self.disconnect_with_error(e);
        }
        if was_empty != self.outbound.is_empty() {
            self.bump_io_generation();
        }
    }

    fn disconnect_with_error(&mut self, _error: Error) {
        debug!("[server] disconnect");

        self.stream.take();
        self.connection_state = ConnectionState::Disconnected;
        self.bump_io_generation();
    }

    fn disconnect(&mut self) {
        debug!("[server] disconnected");

        self.stream.take();
        self.connection_state = ConnectionState::Disconnected;
        self.bump_io_generation();
    }

    fn check_connection_timeout(&mut self) {
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
        }
    }

    fn complete_connection(&mut self) {
        if !matches!(self.connection_state, ConnectionState::Connecting { .. })
        {
            return;
        }

        let Some(ref stream) = self.stream else {
            return;
        };

        match stream.take_error() {
            Ok(None) => {
                self.connection_state = ConnectionState::Connected;
                self.bump_io_generation();
                self.on_connection_established();
            }
            Ok(Some(e)) | Err(e) => {
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

        self.process_read();
    }

    fn io_interest(&self) -> Option<Interest> {
        self.stream.as_ref()?;

        match self.connection_state {
            ConnectionState::Connecting { .. } => Some(Interest::WRITABLE),
            ConnectionState::Connected => {
                let interest = if self.outbound.is_empty() {
                    Interest::READABLE
                } else {
                    Interest::READABLE.add(Interest::WRITABLE)
                };
                Some(interest)
            }
            ConnectionState::Disconnected => None,
        }
    }

    const fn bump_io_generation(&mut self) {
        self.io_generation = self.io_generation.wrapping_add(1);
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
                self.check_connection_timeout();
            }
            ConnectionState::Connected | ConnectionState::Disconnected => {}
        }
        self.check_login_timeout();
    }

    fn handle_io_event(&mut self, _registry: &Registry, event: &Event) {
        if matches!(self.connection_state, ConnectionState::Connecting { .. })
            && event.is_writable()
        {
            self.complete_connection();
        }

        if !matches!(self.connection_state, ConnectionState::Connected) {
            return;
        }

        if event.is_readable() {
            self.process_read();
        }

        if event.is_writable() {
            self.flush_outbound();
        }
    }

    fn io_generation(&self) -> u64 {
        self.io_generation
    }

    fn register_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        let Some(interest) = self.io_interest() else {
            return Ok(false);
        };
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };

        registry.register(stream, token, interest)?;
        Ok(true)
    }

    fn reregister_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        let Some(interest) = self.io_interest() else {
            return Ok(false);
        };
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };

        registry.reregister(stream, token, interest)?;
        Ok(true)
    }

    fn deregister_io(&mut self, registry: &Registry) -> io::Result<()> {
        if let Some(stream) = self.stream.as_mut() {
            registry.deregister(stream)?;
        }
        Ok(())
    }

    fn tick_interval(&self) -> Option<Duration> {
        match (
            &self.connection_state,
            self.pending_login.as_ref().map(|_| ()),
        ) {
            (ConnectionState::Connecting { .. }, _) | (_, Some(())) => {
                Some(Duration::from_secs(1))
            }
            (
                ConnectionState::Connected | ConnectionState::Disconnected,
                None,
            ) => None,
        }
    }
}
