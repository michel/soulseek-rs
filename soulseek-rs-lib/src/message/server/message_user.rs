use crate::actor::server_actor::{ServerMessage, UserMessage};
use crate::info;
use crate::message::{Message, MessageHandler};

use std::sync::mpsc::Sender;

pub struct MessageUser;

impl MessageHandler<ServerMessage> for MessageUser {
    fn get_code(&self) -> u8 {
        22
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerMessage>) {
        let id = message.read_int32();
        let timestamp = message.read_int32();
        let username = message.read_string();
        let message_content = message.read_string();
        let new_message = message.read_bool();
        let user_message = UserMessage::new(
            id,
            timestamp,
            username.clone(),
            message_content,
            new_message,
        );

        info!("[MessageUser] User message received:{:?}", user_message);
        user_message.print()
    }
}
