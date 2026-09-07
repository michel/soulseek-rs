//! `AdminMessage` (server code 66): an announcement from the server to
//! everyone, surfaced as a private message from the user "server".

use crate::actor::server_actor::{ServerMessage, UserMessage};
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct AdminMessageHandler;

impl MessageHandler<ServerMessage> for AdminMessageHandler {
    fn get_code(&self) -> u32 {
        66
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() as u32);
        let announcement = UserMessage::new(
            0,
            now,
            "server".to_string(),
            message.read_string(),
            false,
        );
        let _ =
            sender.send(ServerMessage::PrivateMessageReceived(announcement));
    }
}
