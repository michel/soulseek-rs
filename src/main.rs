mod client;
mod dispatcher;
mod message;
mod peer;
mod server;
mod utils;

use std::{thread::sleep, time::Duration};

use client::Client;
use server::ServerAddress;

fn main() {
    let mut client = Client::new(
        // ServerAddress::new(String::from("localhost"), 2416),
        ServerAddress::new(String::from("server.slsknet.org"), 2416),
        String::from("insane_in_the_brain2"),
        // String::from("invalid"),
        String::from("13375137"),
    );

    client.connect();
    match client.login() {
        Ok(_) => {
            client.search("the weekend", Duration::from_secs(10));
        }
        Err(e) => {
            println!("Failed to login: {}", e);
        }
    }
}
