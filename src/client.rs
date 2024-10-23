use crate::{
    peer::peer::Peer,
    server::{Server, ServerAddress},
    utils::md5,
};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

pub struct Client {
    address: ServerAddress,
    username: String,
    password: String,
    server: Option<Server>,
}

pub enum ClientOperation {
    ConnectToPeer(Peer),
}

impl Client {
    pub fn new(address: ServerAddress, username: String, password: String) -> Self {
        Self {
            address,
            username,
            password,
            server: None,
        }
    }

    pub fn connect(&mut self) {
        let (channel, message_reader): (Sender<ClientOperation>, Receiver<ClientOperation>) =
            mpsc::channel();

        // self.read_form_channel(message_reader);
        self.server = match Server::new(self.address.clone(), channel) {
            Ok(mut server) => {
                println!(
                    "Connected to server at {}:{}",
                    server.get_address().get_host(),
                    server.get_address().get_port()
                );
                Some(server)
            }
            Err(e) => {
                eprintln!("Error connecting to server: {}", e);
                None
            }
        };
    }
    pub fn login(&self) {
        // Attempt to login
        println!("Logging in as {}", self.username);
        if let Some(server) = &self.server {
            if let Err(e) = server.login(&self.username, &self.password) {
                eprintln!("Error during login: {}", e);
            }
        } else {
            println!("Not connected to server");
        }
    }

    pub fn read_form_channel(&mut self, message_reader: Receiver<ClientOperation>) {
        thread::spawn(move || {
            for operation in message_reader.iter() {
                match operation {
                    ClientOperation::ConnectToPeer(peer) => {
                        println!("Received ConnectToPeer operation");
                        peer.print();
                    }
                }
            }
        });
    }

    pub fn search(&self, query: &str, timeout: Duration) {
        println!("Searching for {}", query);
        if let Some(server) = &self.server {
            let hash = md5::md5(query);
            let token = hash[0..8].to_string();
            println!("Token: {}", token);

            server.file_search(&token, &query);

            let start = Instant::now();

            while true {
                if start.elapsed() >= timeout {
                    break;
                }
            }
            println!("search done");
        } else {
            eprintln!("Not connected to server");
        }
    }
}
