use crate::{debug, info};
use std::sync::mpsc::Sender;

use crate::{
    message::handlers::MessageHandler, message::Message,
    server::ServerOperation,
};

pub struct FileSearchHandler;

impl MessageHandler<ServerOperation> for FileSearchHandler {
    fn get_code(&self) -> u32 {
        26
    }
    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        debug!("Handling file search message");
        let username = message.read_string();
        let token = message.read_int32();
        let query = message.read_string();
        info!(
            "Message search username:{}, token: {}, query: {}",
            username, token, query
        );
    }
}
