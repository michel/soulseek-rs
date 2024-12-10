use crate::{
    peer::peer::Peer,
    server::{Server, ServerAddress},
    utils::md5,
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use std::{
    sync::mpsc::{Receiver, Sender},
    thread::sleep,
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
            Ok(server) => {
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
    pub fn login(&self) -> Result<bool, std::io::Error> {
        // Attempt to login
        println!("Logging in as {}", self.username);
        if let Some(server) = &self.server {
            let result = server.login(&self.username, &self.password);
            if result.unwrap() == true {
                println!("Logged in as {}", self.username);
                return Ok(true);
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Error logging in",
                ));
            }
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Not connected to server",
            ));
        }
    }

    // pub fn read_form_channel(&mut self, message_reader: Receiver<ClientOperation>) {
    //     thread::spawn(move || {
    //         for operation in message_reader.iter() {
    //             match operation {
    //                 ClientOperation::ConnectToPeer(peer) => {
    //                     println!("Received ConnectToPeer operation");
    //                     peer.print();
    //                 }
    //             }
    //         }
    //     });
    // }

    pub fn search(&self, query: &str) {
        println!("Searching for {}", query);
        if let Some(server) = &self.server {
            let hash = md5::md5(query);
            let token = hash[0..8].to_string();
            println!("Token: {}", token);

            server.file_search(&token, &query);
        } else {
            eprintln!("Not connected to server");
        }
    }
}
