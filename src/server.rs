use crate::dispatcher::MessageDispatcher;
use crate::message::factory::{build_init_message, build_login_message};
use crate::message::MessageReader;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::thread::{self};
use std::time::Duration;

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
    // pub fn set_host(&mut self, host: String) {
    //     self.host = host;
    // }
    //
    // pub fn set_port(&mut self, port: u16) {
    //     self.port = port;
    // }
    //
    // pub fn set_host_port(&mut self, host: String, port: u16) {
    //     self.host = host;
    //     self.port = port;
    // }
    //
    // pub fn get_host_port(&self) -> String {
    //     format!("{}:{}", self.host, self.port)
    // }
}

pub struct Server {
    address: ServerAddress,
    stream: Arc<Mutex<TcpStream>>,
    message_queue: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
}

#[derive(Debug, Clone)]
pub struct Context {
    message_queue: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
    rooms: Vec<Room>,
}
impl Context {
    pub fn new(message_queue: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>) -> Self {
        Self {
            message_queue,
            rooms: Vec::new(),
        }
    }

    pub fn set_rooms(&mut self, rooms: Vec<Room>) {
        self.rooms = rooms;
    }

    pub fn queue_message(&self, message: Vec<u8>) {
        let (lock, cvar) = &*self.message_queue;
        let mut queue = lock.lock().unwrap();
        println!("Queueing message: {:?}", message); // Debug log before pushing to queue
        queue.push_back(message);
        cvar.notify_one();
    }
}

#[derive(Debug, Clone)]
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
}
impl Server {
    /// Create a new instance of Server, returning a Result
    pub fn new(address: ServerAddress) -> Result<Self, io::Error> {
        println!("Connecting to server at {}:{}", address.host, address.port);

        // Convert the host and port into a socket address
        let socket_address = format!("{}:{}", address.host, address.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid address"))?;

        // Establish the connection with a timeout
        let stream = TcpStream::connect_timeout(&socket_address, Duration::from_secs(10))?;

        // Set the stream to non-blocking mode if needed or configure timeouts
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let message_queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new())); // Initialize the message queue with Condvar

        let server = Self {
            address,
            stream: Arc::new(Mutex::new(stream)),
            message_queue,
        };

        server.start_read_write_loops();

        Ok(server)
    }

    pub fn get_address(&self) -> &ServerAddress {
        &self.address
    }

    /// Start reading and writing loops in separate threads
    fn start_read_write_loops(&self) {
        // Create a channel to send messages to the write thread
        let read_stream = Arc::clone(&self.stream);
        let write_stream = Arc::clone(&self.stream);
        let message_queue = Arc::clone(&self.message_queue);

        let barrier = Arc::new(Barrier::new(2));
        let read_barrier = barrier.clone();
        let write_barrier = barrier.clone();

        let mut buffered_reader = MessageReader::new();
        let mut dispatcher = MessageDispatcher::new(Context::new(Arc::clone(&message_queue)));

        // Spawn a thread to handle reading from the server
        thread::spawn(move || {
            read_barrier.wait();

            loop {
                match buffered_reader.read_from_socket(&mut read_stream.lock().unwrap()) {
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
                        println!("Received message: {:?}", message.get_data());
                        dispatcher.dispatch(&mut message)
                    }
                    Err(e) => {
                        println!("Error extracting message: {}", e)
                    }
                    Ok(None) => continue,
                }
            }
        });

        // introduce a channel , get rid of the thread ?
        thread::spawn(move || {
            write_barrier.wait();

            loop {
                let (lock, cvar) = &*message_queue;
                let mut queue = lock.lock().unwrap();

                // Wait for a message to be added to the queue
                while queue.is_empty() {
                    queue = cvar.wait(queue).unwrap();
                }

                // Pop and send the message
                if let Some(message) = queue.pop_front() {
                    println!("Sending message from queue: {:?}", message); // Debug log when sending
                    match write_stream.lock().unwrap().write_all(&message) {
                        Ok(_) => println!("Sent buffered message: {:?}", message),
                        Err(e) => {
                            eprintln!("Failed to send buffered message: {}", e);
                            // Handle error appropriately
                            break;
                        }
                    }
                }
            }
        });
    }

    fn queue_message(&self, message: Vec<u8>) {
        let context = Context::new(self.message_queue.clone());
        context.queue_message(message);
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<(), std::io::Error> {
        self.queue_message(build_init_message().get_data());
        self.queue_message(build_login_message(username, password).get_data());
        // self.queue_message(build_shared_folders_message(1, 2).get_data());
        Ok(())
    }
}
