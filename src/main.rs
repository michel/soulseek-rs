mod client;
mod dispatcher;
mod message;
mod peer;
mod server;
mod utils;

use std::thread::{self};
use std::time::Duration;

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
    client.login();
    thread::spawn(move || {
        client.search("Trance wax", Duration::from_secs(10));
    });
    thread::sleep(Duration::from_secs(20));
}
