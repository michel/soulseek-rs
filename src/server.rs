use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::factory::{
    build_file_search_message, build_init_message, build_login_message, build_no_parent_message,
    build_set_status_message, build_set_wait_port_message, build_shared_folders_message,
};
use crate::message::{Message, MessageReader};
use crate::peer::listen::Listen;
use crate::peer::peer::Peer;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, sleep, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ServerAddress {
    host: String,
    port: u16,
}

impl ServerAddress {
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
    user_messages: HashMap<String, UserMessage>,
    rooms: Rooms,
}

impl Context {
    pub fn new() -> Self {
        Self {
            rooms: Rooms::new(),
            user_messages: HashMap::new(),
            logged_in: Option::None,
        }
    }

    pub fn add_message_for_user(&mut self, username: String, message: UserMessage) {
        self.user_messages.insert(username.to_string(), message);
    }

    #[allow(dead_code)]
    pub fn get_messages_for_user(&self, username: String) -> Option<&UserMessage> {
        self.user_messages.get(&username)
    }

    pub fn get_rooms(&mut self) -> &mut Rooms {
        &mut self.rooms
    }

    #[cfg(test)]
    pub fn get_user_messages(&self) -> &HashMap<String, UserMessage> {
        &self.user_messages
    }
}
#[derive(Debug, Clone)]
pub struct UserMessage {
    id: i32,
    timestamp: i32,
    username: String,
    message: String,
    new_message: bool,
}
impl UserMessage {
    pub fn new(
        id: i32,
        timestamp: i32,
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
        println!(
            "Timestamp: {}. User: {}, Id: #{}, New message: {} Message: {}",
            self.timestamp, self.username, self.id, self.new_message, self.message
        );
    }
}

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

    pub fn print(&self) {
        println!("Public rooms ({}):", self.public_rooms.len());
        for room in &self.public_rooms {
            room.print();
        }
        println!("Owned private rooms ({}):", self.owned_private_rooms.len());
        for room in &self.owned_private_rooms {
            room.print();
        }
        println!("Private rooms ({}):", self.private_rooms.len());
        for room in &self.private_rooms {
            room.print();
        }
        println!(
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
    pub fn new(name: String, number_of_users: i32) -> Self {
        Self {
            name,
            number_of_users,
        }
    }
    pub fn set_number_of_users(&mut self, number_of_users: i32) {
        self.number_of_users = number_of_users;
    }
    pub fn print(&self) {
        println!(
            "Room: {}, Number of users: {}",
            self.name, self.number_of_users
        );
    }

    // pub fn get_name(&self) -> &str {
    //     &self.name
    // }
    // pub fn get_number_of_users(&self) -> i32 {
    //     self.number_of_users
    // }
}

pub enum ServerOperation {
    // ReceivedMessage(Message),
    LoginStatus(bool),
    SendMessage(Message),
    ConnectToPeer(Peer),
}

#[derive(Debug)]
pub struct Server {
    address: ServerAddress,
    sender: Sender<ServerOperation>,
    context: Arc<Mutex<Context>>,
    read_handle: Option<JoinHandle<()>>,
    monitor_handle: Option<JoinHandle<()>>,
    write_handle: Option<JoinHandle<()>>,
}
impl Server {
    /// Create a new instance of Server, returning a Result
    pub fn new(
        address: ServerAddress,
        client_channel: Sender<ClientOperation>,
    ) -> Result<Self, io::Error> {
        let (sender, server_channel): (Sender<ServerOperation>, Receiver<ServerOperation>) =
            mpsc::channel();

        let context = Arc::new(Mutex::new(Context::new()));

        let mut server = Self {
            address,
            context,
            sender,
            read_handle: None,
            monitor_handle: None,
            write_handle: None,
        };

        server.start_read_write_loops(server_channel)?;
        Ok(server)
    }

    pub fn get_address(&self) -> &ServerAddress {
        &self.address
    }

    /// Start reading and writing loops in separate threads
    fn start_read_write_loops(
        &mut self,
        server_channel: Receiver<ServerOperation>,
    ) -> Result<(), io::Error> {
        let socket_address = format!("{}:{}", self.address.host, self.address.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid address"))?;

        println!(
            "Connecting to server at {}:{}",
            self.address.host, self.address.port
        );

        let stream = TcpStream::connect_timeout(&socket_address, Duration::from_secs(10))?;

        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let mut read_stream = stream.try_clone()?;
        let mut write_stream = stream.try_clone()?;

        let barrier = Arc::new(Barrier::new(3));
        let read_barrier = barrier.clone();
        let write_barrier = barrier.clone();
        let done_barrier = barrier.clone();
        let sender = self.sender.clone();

        let read_handle = thread::spawn(move || {
            read_barrier.wait();
            let dispatcher = MessageDispatcher::new(sender);
            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        println!("Read operation timed out");
                        continue;
                    }
                    Err(e) => {
                        eprintln!("Error reading from server: {}", e);
                        break;
                    }
                }

                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => {
                        println!("Received message: {:?}", message.get_message_code_u32());
                        // message.print_hex();

                        dispatcher.dispatch(&mut message)
                    }
                    Err(e) => {
                        println!("Error extracting message: {}", e)
                    }
                    Ok(None) => continue,
                }
            }
        });

        let monitor_handle = thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(1));
            println!("Stream state: connected={}", stream.peer_addr().is_ok());
        });
        let context = self.context.clone();
        let write_handle = thread::spawn(move || {
            write_barrier.wait();
            loop {
                if let Ok(operation) = server_channel.recv() {
                    match operation {
                        ServerOperation::ConnectToPeer(peer) => peer.print(),
                        ServerOperation::LoginStatus(message) => {
                            context.lock().unwrap().logged_in = Some(message);
                        }
                        ServerOperation::SendMessage(message) => {
                            message.decode();
                            message.print_hex2();
                            match write_stream.write_all(&message.get_buffer()) {
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("Error sending message: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
        done_barrier.wait();
        self.read_handle = Some(read_handle);
        self.monitor_handle = Some(monitor_handle);
        self.write_handle = Some(write_handle);
        Ok(())
    }

    fn queue_message(&self, message: Message) {
        match self.sender.send(ServerOperation::SendMessage(message)) {
            Ok(_) => {}
            Err(e) => println!("Failed to send: {}", e),
        }
    }
    fn start_listener(&self) {
        thread::spawn(move || Listen::new(2234));
    }

    pub fn login(&self, username: &str, password: &str) -> Result<bool, std::io::Error> {
        self.start_listener();
        // Send the login message
        // self.queue_message(build_init_message());
        self.queue_message(build_login_message(username, password));
        let context = self.context.clone();
        let mut logged_in;
        let timeout = Duration::from_secs(5);
        let start = Instant::now();

        // wait till server says your logged in or not
        loop {
            if start.elapsed() > timeout {
                println!("Timeout waiting for login response");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timeout waiting for login response",
                ));
            }

            {
                logged_in = context.lock().unwrap().logged_in.clone()
            }

            if !logged_in.is_none() {
                break;
            }
        }

        if logged_in.unwrap() {
            println!("Logged in as {}", username);
            // self.queue_message(build_set_wait_port_message());
            // self.queue_message(build_shared_folders_message(1, 1));
            // self.queue_message(build_no_parent_message());
            // self.queue_message(build_set_status_message(2));
        }

        Ok(logged_in.unwrap())
    }

    pub fn file_search(&self, token: u32, query: &str) {
        self.queue_message(build_file_search_message(token, query));
    }
}
