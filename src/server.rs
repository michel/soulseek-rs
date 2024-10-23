use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::factory::{build_file_search_message, build_init_message, build_login_message};
use crate::message::{Message, MessageReader};
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self};
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
    pub logged_in: bool,
    pub server_channel: Sender<Message>,
    pub client_channel: Sender<ClientOperation>,
    user_messages: HashMap<String, UserMessage>,
    rooms: Rooms,
}

impl Context {
    pub fn new(server_channel: Sender<Message>, client_channel: Sender<ClientOperation>) -> Self {
        Self {
            server_channel,
            client_channel,
            rooms: Rooms::new(),
            user_messages: HashMap::new(),
            logged_in: false,
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

    pub fn queue_message(&self, message: Message) {
        self.server_channel.send(message).unwrap();
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

pub struct Server {
    address: ServerAddress,
    context: Arc<Mutex<Context>>,
}
impl Server {
    /// Create a new instance of Server, returning a Result
    pub fn new(
        address: ServerAddress,
        client_channel: Sender<ClientOperation>,
    ) -> Result<Self, io::Error> {
        let (message_sender, server_channel): (Sender<Message>, Receiver<Message>) =
            mpsc::channel();

        let context = Arc::new(Mutex::new(Context::new(message_sender, client_channel)));

        let server = Self { address, context };

        server.start_read_write_loops(server_channel)?;
        Ok(server)
    }

    pub fn get_address(&self) -> &ServerAddress {
        &self.address
    }

    /// Start reading and writing loops in separate threads
    fn start_read_write_loops(&self, message_reader: Receiver<Message>) -> Result<(), io::Error> {
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

        let barrier = Arc::new(Barrier::new(2));
        let read_barrier = barrier.clone();
        let write_barrier = barrier.clone();
        let self_context = Arc::clone(&self.context);

        thread::spawn(move || {
            read_barrier.wait();
            let dispatcher = MessageDispatcher::new(self_context);
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
                        // println!("Received message: {:?}", message.get_data());

                        dispatcher.dispatch(&mut message)
                    }
                    Err(e) => {
                        println!("Error extracting message: {}", e)
                    }
                    Ok(None) => continue,
                }
            }
        });

        thread::spawn(move || {
            write_barrier.wait();

            loop {
                let message = message_reader.recv().unwrap();
                // println!("Sending message from queue: {:?}", message); // Debug log when sending
                match write_stream.write_all(&message.get_data()) {
                    // Ok(_) => println!("Sent buffered message: {:?}", message),
                    Err(e) => {
                        eprintln!("Failed to send buffered message: {}", e);
                        break;
                    }
                    Ok(_) => {}
                }
            }
        });
        Ok(())
    }

    fn queue_message(&self, message: Message) {
        self.context.lock().unwrap().queue_message(message);
    }

    pub fn login(&self, username: &str, password: &str) -> Result<(), std::io::Error> {
        self.queue_message(build_init_message());
        self.queue_message(build_login_message(username, password));
        Ok(())
    }

    pub fn file_search(&self, token: &str, query: &str) {
        self.queue_message(build_file_search_message(token, query));
    }
}
