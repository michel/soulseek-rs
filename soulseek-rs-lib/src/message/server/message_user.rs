use crate::actor::server_actor::{ServerMessage, UserMessage};
use crate::info;
use crate::message::server::MessageFactory;
use crate::message::{Message, MessageHandler};

use std::sync::mpsc::Sender;

pub struct MessageUser;

impl MessageHandler<ServerMessage> for MessageUser {
    fn get_code(&self) -> u8 {
        22
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let id = message.read_int32();
        let timestamp = message.read_int32();
        let username = message.read_string();
        let message_content = message.read_string();
        let new_message = message.read_bool();
        let user_message = UserMessage::new(
            id,
            timestamp,
            username,
            message_content,
            new_message,
        );

        info!("[MessageUser] User message received:{:?}", user_message);
        user_message.print();

        // Acknowledge freshly delivered messages so the server does not keep
        // re-sending them on every reconnect.
        if new_message {
            let _ = sender.send(ServerMessage::SendMessage(
                MessageFactory::build_message_acked(id),
            ));
        }

        // Surface the message to the client so it can be read via the API.
        let _ =
            sender.send(ServerMessage::PrivateMessageReceived(user_message));
    }
}
