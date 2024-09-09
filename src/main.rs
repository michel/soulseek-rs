mod client;
mod dispatcher;
mod message;
mod server;

use std::thread;

use client::Client;
use server::ServerAddress;

fn main() {
    let client = Client::new(
        // ServerAddress::new(String::from("localhost"), 2416),
        ServerAddress::new(String::from("server.slsknet.org"), 2416),
        String::from("insane_in_the_brain2"),
        // String::from("invalid"),
        String::from("13375137"),
    );
    client.connect();
    loop {
        thread::park();
    }
}
