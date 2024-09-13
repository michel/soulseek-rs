mod client;
mod dispatcher;
mod message;
mod peer;
mod server;
mod utils;

use client::Client;
use server::PeerAddress;
use std::time::Duration;

fn main() {
    let mut client = Client::new(
        // ServerAddress::new(String::from("localhost"), 2416),
        PeerAddress::new(String::from("server.slsknet.org"), 2416),
        String::from("insane_in_the_brain2"),
        // String::from("invalid"),
        String::from("13375137"),
    );

    client.connect();
    match client.login() {
        Ok(_) => {
            client.search("the weekend", Duration::from_secs(20));
        }
        Err(e) => {
            println!("Failed to login: {}", e);
        }
    }
}
