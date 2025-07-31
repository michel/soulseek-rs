use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::server::ConnectToPeerHandler;
use crate::message::server::ExcludedSearchPhrasesHandler;
use crate::message::server::FileSearchHandler;
use crate::message::server::LoginHandler;
use crate::message::server::MessageFactory;
use crate::message::server::MessageUser;
use crate::message::server::ParentMinSpeedHandler;
use crate::message::server::ParentSpeedRatioHandler;
use crate::message::server::PrivilegedUsersHandler;
use crate::message::server::RoomListHandler;
use crate::message::server::WishListIntervalHandler;
use crate::message::Handlers;
use crate::message::Message;
use crate::message::MessageReader;
use crate::peer::listen::Listen;
use crate::peer::ConnectionType;
use crate::peer::Peer;

use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self};
use std::time::{Duration, Instant};

use crate::{debug, error, info, warn};

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

    // pub fn get_messages_for_user(&self, username: String) -> Option<&UserMessage> {
    //     self.user_messages.get(&username)
    // }
    //
    // pub fn get_rooms(&mut self) -> &mut Rooms {
    //     &mut self.rooms
    // }
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
}

#[derive(Debug)]
pub struct Server {
    address: PeerAddress,
    sender: Sender<ServerOperation>,
    context: Arc<Mutex<Context>>,
}
impl Server {
    /// Create a new instance of Server, returning a Result
    pub fn new(
        address: PeerAddress,
        client_channel: Sender<ClientOperation>,
    ) -> Result<Self, io::Error> {
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
    ) -> Result<(), io::Error> {
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
            handlers.register_handler(ConnectToPeerHandler);

            let dispatcher = MessageDispatcher::new(sender, handlers);

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
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

                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => {
                        // trace!(
                        //     "[server] ← {:?}",
                        //     message
                        //         .get_message_name(
                        //             MessageType::Server,
                        //             message.get_message_code() as u32
                        //         )
                        //         .map_err(|e| e.to_string())
                        // );

                        dispatcher.dispatch(&mut message)
                    }
                    Err(e) => {
                        warn!("Error extracting message: {}", e)
                    }
                    Ok(None) => continue,
                }
            }
        });

        // let monitor_handle = thread::spawn(move || loop {
        //     thread::sleep(Duration::from_secs(1));
        //     println!("Stream state: connected={}", stream.peer_addr().is_ok());
        // });
        let context = self.context.clone();
        thread::spawn(move || {
            write_barrier.wait();
            loop {
                if let Ok(operation) = server_channel.recv() {
                    match operation {
                        ServerOperation::ConnectToPeer(peer) => {
                            match peer.connection_type {
                                ConnectionType::P => {
                                    match client_channel.send(
                                        ClientOperation::ConnectToPeer(peer),
                                    ) {
                                        Ok(_) => {}
                                        Err(_e) => {}
                                    }
                                }
                                ConnectionType::F => {
                                    match client_channel.send(
                                        ClientOperation::PierceFireWall(peer),
                                    ) {
                                        Ok(_) => {
                                            debug!("Sent PierceFireWall operation for F-type connection");
                                        }
                                        Err(_e) => {
                                            error!("Failed to send PierceFireWall operation");
                                        }
                                    }
                                }
                                ConnectionType::D => {}
                            }
                        }
                        ServerOperation::LoginStatus(message) => {
                            context.lock().unwrap().logged_in = Some(message);
                        }
                        ServerOperation::PierceFirewall(token) => {
                            let pierce_message =
                                MessageFactory::build_pierce_firewall_message(
                                    token,
                                );
                            match write_stream
                                .write_all(&pierce_message.get_buffer())
                            {
                                Ok(_) => {
                                    debug!("Sent PierceFirewall message with token: {}", token);
                                }
                                Err(e) => {
                                    error!("Error writing PierceFirewall message: {}", e);
                                    break;
                                }
                            }
                        }
                        ServerOperation::SendMessage(message) => {
                            // trace!(
                            //     "[server] ➡ {:?}",
                            //     message
                            //         .get_message_name(
                            //             MessageType::Server,
                            //             u32::from_le_bytes(
                            //                 message.get_slice(0, 4).try_into().unwrap()
                            //             )
                            //         )
                            //         .map_err(|e| e.to_string()),
                            // );
                            match write_stream.write_all(&message.get_buffer())
                            {
                                Ok(_) => {}
                                Err(e) => {
                                    error!(
                                        "Error writing message to stream : {}",
                                        e
                                    );
                                    break;
                                }
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
    fn start_listener(&self, server_channel: Sender<ServerOperation>) {
        thread::spawn(move || Listen::start(2234, server_channel));
    }

    pub fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<bool, std::io::Error> {
        self.start_listener(self.sender.clone());
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
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timeout waiting for login response",
                ));
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
            self.queue_message(MessageFactory::build_set_wait_port_message());
            self.queue_message(MessageFactory::build_shared_folders_message(
                1, 499,
            ));
            self.queue_message(MessageFactory::build_no_parent_message());
            self.queue_message(MessageFactory::build_set_status_message(2));
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
