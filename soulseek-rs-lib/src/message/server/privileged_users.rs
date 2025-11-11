use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct PrivilegedUsersHandler;

impl MessageHandler<ServerMessage> for PrivilegedUsersHandler {
    fn get_code(&self) -> u8 {
        69
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerMessage>) {
        let number = message.read_int32();
        debug!("Number of privileged users: {}", number);
    }
}
