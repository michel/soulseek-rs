use crate::actor::{Actor, ActorHandle, ConnectionState};
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

use std::io::{self, Error, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::{debug, error, info, trace, warn, SoulseekRs};

#[derive(Debug, Clone)]
pub struct PeerAddress {
    host: String,
    port: u16,
}

impl PeerAddress {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub fn get_host(&self) -> &str {
        &self.host
    }

    pub fn get_port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug)]
pub struct Context {
    pub logged_in: Option<bool>,
    #[allow(dead_code)]
    rooms: Rooms,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    pub fn new() -> Self {
        Self {
            #[allow(dead_code)]
            rooms: Rooms::new(),
            logged_in: Option::None,
        }
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
    pub fn new(
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
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct Rooms {
    pub public_rooms: Vec<Room>,
    pub owned_private_rooms: Vec<Room>,
    pub private_rooms: Vec<Room>,
    pub operated_private_rooms: Vec<Room>,
}
impl Rooms {
    fn new() -> Self {
        Self {
            public_rooms: Vec::new(),
            owned_private_rooms: Vec::new(),
            private_rooms: Vec::new(),
            operated_private_rooms: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn print(&self) {
        info!("Public rooms ({}):", self.public_rooms.len());
        for room in &self.public_rooms {
            room.print();
        }
        info!("Owned private rooms ({}):", self.owned_private_rooms.len());
        for room in &self.owned_private_rooms {
            room.print();
        }
        info!("Private rooms ({}):", self.private_rooms.len());
        for room in &self.private_rooms {
            room.print();
        }
        info!(
            "Operated private rooms ({}):",
            self.operated_private_rooms.len()
        );
        for room in &self.operated_private_rooms {
            room.print();
        }
    }
}
#[derive(Debug)]
pub struct Room {
    name: String,
    number_of_users: i32,
}
impl Room {
    #[allow(dead_code)]
    pub fn new(name: String, number_of_users: i32) -> Self {
        Self {
            name,
            number_of_users,
        }
    }
    #[allow(dead_code)]
    pub fn set_number_of_users(&mut self, number_of_users: i32) {
        self.number_of_users = number_of_users;
    }
    pub fn print(&self) {
        debug!(
            "Room: {}, Number of users: {}",
            self.name, self.number_of_users
        );
    }
}

#[derive(Debug, Clone)]
pub enum ServerOperation {
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
}

pub struct ServerActor {
    address: PeerAddress,
    context: Arc<RwLock<Context>>,
    listen_port: u32,
    enable_listen: bool,
    stream: Option<TcpStream>,
    connection_state: ConnectionState,
    reader: MessageReader,
    client_channel: Sender<ClientOperation>,
    self_handle: Option<ActorHandle<ServerOperation>>,
    dispatcher: Option<MessageDispatcher<ServerOperation>>,
    dispatcher_receiver: Option<Receiver<ServerOperation>>,
    dispatcher_sender: Option<Sender<ServerOperation>>,
    queued_messages: Vec<ServerOperation>,
}

impl ServerActor {
    pub fn new(
        address: PeerAddress,
        client_channel: Sender<ClientOperation>,
        listen_port: u32,
        enable_listen: bool,
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
        }
    }

    pub fn get_address(&self) -> &PeerAddress {
        &self.address
    }

    pub fn get_sender(&self) -> Option<&Sender<ServerOperation>> {
        self.dispatcher_sender.as_ref()
    }

    fn initiate_connection(&mut self) -> bool {
        let host = self.address.host.clone();
        let port = self.address.port;

        let addr_str = format!("{}:{}", host, port);

        let mut socket_addrs = match addr_str.to_socket_addrs() {
            Ok(addrs) => addrs,
            Err(e) => {
                error!("[server_actor] Failed to resolve address: {}", e);

                self.disconnect_with_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    e,
                ));
                return false;
            }
        };

        let socket_addr = socket_addrs.next();

        match socket_addr {
            Some(addr) => {
                if let Ok(stream) = TcpStream::connect(addr) {
                    if let Err(e) = stream.set_nonblocking(true) {
                        error!(
                            "[server_actor] Failed to set non-blocking: {}",
                            e
                        );
                        self.disconnect_with_error(e);
                        return false;
                    }

                    stream.set_nodelay(true).ok();

                    self.stream = Some(stream);
                    self.connection_state = ConnectionState::Connecting {
                        since: Instant::now(),
                    };
                    true
                } else {
                    match TcpStream::connect(addr) {
                        Ok(stream) => {
                            if let Err(e) = stream.set_nonblocking(true) {
                                error!("[server_actor] Failed to set non-blocking: {}", e);
                                self.disconnect_with_error(e);
                                return false;
                            }
                            stream.set_nodelay(true).ok();
                            self.stream = Some(stream);
                            self.connection_state =
                                ConnectionState::Connecting {
                                    since: Instant::now(),
                                };
                            true
                        }
                        Err(e) => {
                            self.disconnect_with_error(e);
                            false
                        }
                    }
                }
            }
            None => {
                let error_msg =
                    format!("No socket addresses found for {}:{}", host, port);
                error!("[server_actor] {}", error_msg);
                self.disconnect_with_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    error_msg,
                ));
                false
            }
        }
    }

    pub fn set_self_handle(&mut self, handle: ActorHandle<ServerOperation>) {
        self.self_handle = Some(handle);
    }

    fn initialize_dispatcher(&mut self) {
        let (dispatcher_sender, dispatcher_receiver) =
            std::sync::mpsc::channel::<ServerOperation>();

        self.dispatcher_receiver = Some(dispatcher_receiver);
        self.dispatcher_sender = Some(dispatcher_sender.clone());

        self.client_channel
            .send(ClientOperation::SetServerSender(dispatcher_sender.clone()))
            .unwrap();

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

        self.dispatcher = Some(MessageDispatcher::new(
            "server_actor".into(),
            dispatcher_sender,
            handlers,
        ));
    }

    fn process_dispatcher_messages(&mut self) {
        let messages: Vec<ServerOperation> = self
            .dispatcher_receiver
            .as_ref()
            .map_or_else(Vec::new, |receiver| {
                let mut msgs = Vec::new();
                while let Ok(msg) = receiver.try_recv() {
                    msgs.push(msg);
                }
                msgs
            });

        messages
            .iter()
            .for_each(|msg| self.handle_message(msg.clone()));
    }

    pub fn login(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<bool, SoulseekRs> {
        self.queue_message(MessageFactory::build_login_message(
            username, password,
        ));
        let context = self.context.clone();
        let mut logged_in;
        let timeout = Duration::from_secs(5);
        let start = Instant::now();

        loop {
            if start.elapsed() > timeout {
                warn!("Timeout waiting for login response");
                return Err(SoulseekRs::Timeout);
            }

            {
                logged_in = context.read().unwrap().logged_in
            }

            if logged_in.is_some() {
                break;
            }
        }

        if logged_in.unwrap() {
            info!("Logged in as {}", username);
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
        }

        Ok(logged_in.unwrap())
    }

    pub fn file_search(&mut self, token: u32, query: &str) {
        self.queue_message(MessageFactory::build_file_search_message(
            token, query,
        ));
    }

    fn handle_message(&mut self, msg: ServerOperation) {
        if !matches!(self.connection_state, ConnectionState::Connected) {
            match &msg {
                ServerOperation::ProcessRead => {
                    // Always process read operations
                }
                _ => {
                    // Queue all other messages when not connected
                    self.queued_messages.push(msg);
                    return;
                }
            }
        }

        match msg {
            ServerOperation::ConnectToPeer(peer) => {
                if let Some(op) = match peer.connection_type {
                    ConnectionType::P | ConnectionType::F => {
                        Some(ClientOperation::ConnectToPeer(peer.clone()))
                    }
                    ConnectionType::D => None,
                } {
                    self.client_channel.send(op).unwrap();
                }
            }
            ServerOperation::LoginStatus(message) => {
                self.context.write().unwrap().logged_in = Some(message);
            }
            ServerOperation::PierceFirewall(token) => {
                self.send_message(
                    MessageFactory::build_pierce_firewall_message(token),
                );
            }
            ServerOperation::SendMessage(message) => {
                self.send_message(message);
            }
            ServerOperation::GetPeerAddress(username) => {
                self.send_message(MessageFactory::build_get_peer_address(
                    &username,
                ));
            }
            ServerOperation::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            } => {
                debug!(
                    "[server_actor] Received GetPeerAddress response for {}: {}:{} (obf_type: {}, obf_port: {})"
                    , username, host, port, obfuscation_type, obfuscated_port
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
                    error!("[server_actor] Error forwarding GetPeerAddress response to client: {}", e);
                }
            }
            ServerOperation::ProcessRead => {
                self.process_read();
            }
            ServerOperation::Login {
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
                std::thread::spawn(move || loop {
                    if start.elapsed() >= timeout {
                        let _ = response.send(Err(SoulseekRs::Timeout));
                        break;
                    }

                    if let Some(logged_in) = context.read().unwrap().logged_in {
                        let result = if logged_in {
                            Ok(true)
                        } else {
                            Err(SoulseekRs::AuthenticationFailed)
                        };
                        let _ = response.send(result);
                        break;
                    }

                    std::thread::sleep(Duration::from_millis(100));
                });
            }
            ServerOperation::FileSearch { token, query } => {
                self.file_search(token, &query);
            }
        }
    }

    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages();
        }

        {
            let stream = match self.stream.as_mut() {
                Some(s) => s,
                None => return,
            };

            match self.reader.read_from_socket(stream) {
                Ok(()) => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    debug!("[server_actor] Read operation timed out",);
                }
                Err(e) => {
                    error!(
                        "[server_actor] Error reading from server: {} (kind: {:?}). Disconnecting.",
                         e, e.kind()
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
                        "[server_actor] ← Message #{}: {:?}",
                        extracted_count,
                        message
                            .get_message_name(
                                MessageType::Server,
                                message.get_message_code() as u32
                            )
                            .map_err(|e| e.to_string())
                    );
                    if let Some(ref dispatcher) = self.dispatcher {
                        dispatcher.dispatch(&mut message);
                    } else {
                        warn!("[server_actor] No dispatcher available!",);
                    }
                }
                Err(e) => {
                    warn!( "[server_actor] Error extracting message: {}. Disconnecting.", e);
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

    fn queue_message(&mut self, message: Message) {
        if let Some(sender) = &self.dispatcher_sender {
            match sender.send(ServerOperation::SendMessage(message)) {
                Ok(_) => {}
                Err(e) => error!("Failed to send: {}", e),
            }
        } else {
            self.queued_messages
                .push(ServerOperation::SendMessage(message));
        }
    }

    fn send_message(&mut self, message: Message) {
        let stream = match self.stream.as_mut() {
            Some(s) => s,
            None => {
                error!("[server_actor] Cannot send message: stream is None");
                return;
            }
        };

        trace!(
            "[server_actor] ➡ {:?}",
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
            error!(
                "[server_actor] Error writing message: {}. Disconnecting.",
                e
            );
            self.disconnect_with_error(e);
            return;
        }

        if let Err(e) = stream.flush() {
            error!(
                "[server_actor] Error flushing stream: {}. Disconnecting.",
                e
            );
            self.disconnect_with_error(e);
        }
    }

    fn disconnect_with_error(&mut self, _error: Error) {
        debug!("[server_actor] disconnect");

        self.stream.take();
    }

    fn disconnect(&mut self) {
        debug!("[server_actor] disconnected");

        self.stream.take();
    }

    fn check_connection_status(&mut self) {
        let ConnectionState::Connecting { since } = self.connection_state
        else {
            return;
        };

        if since.elapsed() > Duration::from_secs(20) {
            error!("[server_actor] Connection timeout after 20 seconds");
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
                error!("[server_actor] Connection failed: {}", e);
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
            handle.send(ServerOperation::ProcessRead).ok();
        }

        self.process_read();
    }
}

impl Actor for ServerActor {
    type Message = ServerOperation;

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
        trace!("[server_actor] actor stopping");
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
