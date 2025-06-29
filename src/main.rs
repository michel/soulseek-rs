mod client;
mod dispatcher;
mod message;
mod peer;
mod server;
mod types;
mod utils;

use client::Client;
use server::PeerAddress;
use std::time::Duration;

// PeerAddress::new(String::from("server.slsknet.org"), 2416),
fn main() {
    let mut client = Client::new(
        // ServerAddress::new(String::from("localhost"), 2416),
        // PeerAddress::new(String::from("127.0.0.1"), 2242),
        PeerAddress::new(String::from("server.slsknet.org"), 2242),
        String::from("insane_in_the_brain3"),
        // String::from("invalid"),
        String::from("13375137"),
    );

    client.connect();
    match client.login() {
        Ok(_) => {
            let results = client.search("Fantazia", Duration::from_secs(10));
            println!("Search results: {:?}", results);
        }
        Err(e) => {
            println!("Failed to login: {}", e);
        }
    }
}
