use crate::server::{Server, ServerAddress};

pub struct Client {
    address: ServerAddress,
    username: String,
    password: String,
}

impl Client {
    pub fn new(address: ServerAddress, username: String, password: String) -> Self {
        Self {
            address,
            username,
            password,
        }
    }

    pub fn connect(&self) {
        // Attempt to create a server instance
        match Server::new(self.address.clone()) {
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
            }
            Err(e) => eprintln!("Error connecting to server: {}", e),
        }
    }
}
