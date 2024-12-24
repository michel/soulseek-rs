use std::sync::mpsc::Sender;

use crate::{message::Message, server::ServerOperation};

use super::handlers::MessageHandler;

pub struct FileSearch;

impl MessageHandler for FileSearch {
    fn get_code(&self) -> u8 {
        26
    }
    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        println!("Handling file search message");
        let username = message.read_string();
        let token = message.read_int32();
        let query = message.read_string();
        println!(
            "Message search username:{}, token: {}, query: {}",
            username, token, query
        );
    }
}
