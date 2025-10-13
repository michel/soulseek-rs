use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::error::{Result, SoulseekRs};
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
use crate::message::Message;
use crate::message::MessageReader;
use crate::message::{Handlers, MessageType};
use crate::peer::ConnectionType;
use crate::peer::Peer;

use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self};
use std::time::{Duration, Instant};

use crate::{debug, error, info, trace, warn};

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

pub enum ServerOperation {
    LoginStatus(bool),
    SendMessage(Message),
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

#[derive(Debug)]
pub struct Server {
    address: PeerAddress,
    sender: Sender<ServerOperation>,
    context: Arc<Mutex<Context>>,
}
impl Server {
    pub fn new(
        address: PeerAddress,
        client_channel: Sender<ClientOperation>,
    ) -> std::result::Result<Self, io::Error> {
        let (sender, server_channel): (
            Sender<ServerOperation>,
            Receiver<ServerOperation>,
        ) = mpsc::channel();

        let context = Arc::new(Mutex::new(Context::new()));

        let mut server = Self {
            address,
            context,
            sender,
        };

        server.start_read_write_loops(server_channel, client_channel)?;
        Ok(server)
    }

    pub fn get_address(&self) -> &PeerAddress {
        &self.address
    }

    pub fn get_sender(&self) -> &Sender<ServerOperation> {
        &self.sender
    }

    /// Start reading and writing loops in separate threads
    fn start_read_write_loops(
        &mut self,
        server_channel: Receiver<ServerOperation>,
        client_channel: Sender<ClientOperation>,
    ) -> std::result::Result<(), io::Error> {
        let socket_address =
            format!("{}:{}", self.address.host, self.address.port)
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid address",
                    )
                })?;

        info!(
            "Connecting to server at {}:{}",
            self.address.host, self.address.port
        );

        let stream = TcpStream::connect_timeout(
            &socket_address,
            Duration::from_secs(10),
        )?;

        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;

        let mut read_stream = stream.try_clone()?;
        let mut write_stream = stream.try_clone()?;

        let barrier = Arc::new(Barrier::new(3));
        let read_barrier = barrier.clone();
        let write_barrier = barrier.clone();
        let done_barrier = barrier.clone();
        let sender = self.sender.clone();

        thread::spawn(move || {
            read_barrier.wait();

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

            let dispatcher =
                MessageDispatcher::new("server".to_string(), sender, handlers);

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        debug!("Read operation timed out");
                        continue;
                    }
                    Err(e) => {
                        error!("Error reading from server: {}", e);
                        break;
                    }
                }

                // Extract all available messages from buffer
                loop {
                    match buffered_reader.extract_message() {
                        Ok(Some(mut message)) => {
                            trace!(
                                "[server] ← {:?} - {}",
                                message
                                    .get_message_name(
                                        MessageType::Server,
                                        message.get_message_code_u32()
                                    )
                                    .map_err(|e| e.to_string()),
                                message.get_message_code_u32()
                            );
                            dispatcher.dispatch(&mut message)
                        }
                        Err(e) => {
                            warn!("Error extracting message: {}", e);
                            break;
                        }
                        Ok(None) => break,
                    }
                }
            }
        });

        let context = self.context.clone();
        thread::spawn(move || {
            write_barrier.wait();
            loop {
                if let Ok(operation) = server_channel.recv() {
                    match operation {
                        ServerOperation::ConnectToPeer(peer) => {
                            debug!(
                                "[server] ConnectToPeer {} - ConnectionType {}",
                                peer.username, peer.connection_type
                            );

                            if let Some(op) = match peer.connection_type {
                                ConnectionType::P => {
                                    Some(ClientOperation::ConnectToPeer(peer))
                                }
                                ConnectionType::F => {
                                    Some(ClientOperation::PierceFireWall(peer))
                                }
                                ConnectionType::D => None,
                            } {
                                client_channel.send(op).unwrap();
                            }
                        }
                        ServerOperation::LoginStatus(message) => {
                            context.lock().unwrap().logged_in = Some(message);
                        }
                        ServerOperation::PierceFirewall(token) => {
                            if let Err(e) = write_stream.write_all(
                                &MessageFactory::build_pierce_firewall_message(
                                    token,
                                )
                                .get_buffer(),
                            ) {
                                error!(
                                    "Error writing PierceFirewall message: {}",
                                    e
                                );
                                break;
                            }
                            debug!(
                                "Sent PierceFirewall message with token: {}",
                                token
                            );
                        }
                        ServerOperation::SendMessage(message) => {
                            match write_stream.write_all(&message.get_buffer())
                            {
                                Ok(_) => trace!(
                                    "[server] → {:?} - {:}",
                                    message
                                        .get_message_name(
                                            MessageType::Server,
                                            message.get_message_code_send()
                                                as u32
                                        )
                                        .map_err(|e| e.to_string()),
                                    message.get_message_code_send() as u32
                                ),
                                Err(e) => error!(
                                    "Error writing message to server: {}",
                                    e
                                ),
                            }
                        }
                        ServerOperation::GetPeerAddress(username) => {
                            if let Err(e) = write_stream.write_all(
                                &MessageFactory::build_get_peer_address(
                                    &username,
                                )
                                .get_buffer(),
                            ) {
                                error!("Error writing get_peeer_address message: {}", e);
                                break;
                            }
                        }
                        ServerOperation::GetPeerAddressResponse {
                            username,
                            host,
                            port,
                            obfuscation_type,
                            obfuscated_port,
                        } => {
                            debug!(
                                "Received GetPeerAddress response for {}: {}:{} (obf_type: {}, obf_port: {})",
                                username, host, port, obfuscation_type, obfuscated_port
                            );

                            // Forward the response to the client channel
                            if let Err(e) = client_channel.send(
                                ClientOperation::GetPeerAddressResponse {
                                    username,
                                    host,
                                    port,
                                    obfuscation_type,
                                    obfuscated_port,
                                },
                            ) {
                                error!("Error forwarding GetPeerAddress response to client: {}", e);
                            }
                        }
                    }
                }
            }
        });
        done_barrier.wait();
        Ok(())
    }

    fn queue_message(&self, message: Message) {
        match self.sender.send(ServerOperation::SendMessage(message)) {
            Ok(_) => {}
            Err(e) => error!("Failed to send: {}", e),
        }
    }
    pub fn login(&self, username: &str, password: &str) -> Result<bool> {
        // Send the login message
        self.queue_message(MessageFactory::build_login_message(
            username, password,
        ));
        let context = self.context.clone();
        let mut logged_in;
        let timeout = Duration::from_secs(5);
        let start = Instant::now();

        // wait till server says your logged in or not
        loop {
            if start.elapsed() > timeout {
                warn!("Timeout waiting for login response");
                return Err(SoulseekRs::Timeout);
            }

            {
                logged_in = context.lock().unwrap().logged_in
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
            self.queue_message(MessageFactory::build_set_wait_port_message(
                2235,
            ));
        }

        Ok(logged_in.unwrap())
    }

    pub fn file_search(&self, token: u32, query: &str) {
        self.queue_message(MessageFactory::build_file_search_message(
            token, query,
        ));
    }

    #[allow(dead_code)]
    pub fn pierce_firewall(&self, token: u32) {
        match self.sender.send(ServerOperation::PierceFirewall(token)) {
            Ok(_) => {}
            Err(e) => error!("Failed to send PierceFirewall: {}", e),
        }
    }
}
