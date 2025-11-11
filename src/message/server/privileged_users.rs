use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    actor::server_actor::ServerOperation,
};

pub struct PrivilegedUsersHandler;

impl MessageHandler<ServerOperation> for PrivilegedUsersHandler {
    fn get_code(&self) -> u8 {
        69
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let number = message.read_int32();
        debug!("Number of privileged users: {}", number);
    }
}
