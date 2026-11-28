use crate::{
    actor::server_actor::ServerMessage, debug, info, message::Message,
};
use std::sync::mpsc::Sender;

use crate::message::MessageHandler;

pub struct LoginHandler;

impl MessageHandler<ServerMessage> for LoginHandler {
    fn get_code(&self) -> u32 {
        1
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let response = message.read_int8();

        if response != 1 {
            let reason = message.read_string();
            let detail = (message.get_pointer() < message.get_size())
                .then(|| message.read_string());
            let _ = sender.send(if reason.is_empty() {
                ServerMessage::LoginStatus(false)
            } else {
                ServerMessage::LoginRejected { reason, detail }
            });
            return;
        }

        info!("Login successful");
        let greeting = message.read_string();
        debug!("Server greeting: {:?}", greeting);

        let _ = sender.send(ServerMessage::LoginStatus(true));
    }
}
