mod client;
mod dispatcher;
mod message;
mod peer;
mod server;
mod utils;

use client::Client;
use server::ServerAddress;

fn main() {
    let mut client = Client::new(
        // ServerAddress::new(String::from("localhost"), 2416),
        ServerAddress::new(String::from("server.slsknet.org"), 2416),
        String::from("insane_in_the_brain2"),
        // String::from("invalid"),
        String::from("13x75137"),
    );

    client.connect();
    match client.login() {
        Ok(_) => {
            println!("Logged in successfully");
            client.search("trance wax");
        }
        Err(e) => {
            println!("Failed to login: {}", e);
        }
    }

    // thread::sleep(Duration::from_secs(5));
}
