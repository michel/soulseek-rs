use crate::{
    actor::server_actor::ServerMessage, debug, info, message::Message,
};
use std::sync::mpsc::Sender;

use crate::message::MessageHandler;

pub struct LoginHandler;

impl MessageHandler<ServerMessage> for LoginHandler {
    fn get_code(&self) -> u8 {
        1
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let response = message.read_int8();

        if response != 1 {
            let _ = sender.send(ServerMessage::LoginStatus(false));
            return;
        }

        info!("Login successful");
        let greeting = message.read_string();
        debug!("Server greeting: {:?}", greeting);

        let _ = sender.send(ServerMessage::LoginStatus(true));
    }
}
