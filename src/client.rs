use crate::{
    server::{Server, ServerAddress},
    utils::md5,
};

pub struct Client {
    address: ServerAddress,
    username: String,
    password: String,
    server: Option<Server>,
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
        self.server = match Server::new(self.address.clone()) {
            Ok(mut server) => {
                println!(
                    "Connected to server at {}:{}",
                    server.get_address().get_host(),
                    server.get_address().get_port()
                );

                // Attempt to login
                println!("Logging in as {}", self.username);

                if let Err(e) = server.login(&self.username, &self.password) {
                    eprintln!("Error during login: {}", e);
                }
                Some(server)
            }
            Err(e) => {
                eprintln!("Error connecting to server: {}", e);
                None
            }
        };
    }
    pub fn search(&self, query: &str) {
        if let Some(server) = &self.server {
            let hash = md5::md5(query);
            let token = hash[0..8].to_string();
            server.file_search(&token, &query)
        } else {
            eprintln!("Not connected to server");
        }
    }
}
