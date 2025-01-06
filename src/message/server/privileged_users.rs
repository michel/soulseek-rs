use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    server::ServerOperation,
};

pub struct PrivilegedUsersHandler;

impl MessageHandler<ServerOperation> for PrivilegedUsersHandler {
    fn get_code(&self) -> u8 {
        69
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let number = message.read_int32();
        println!("Number of privilaged users: {}", number);
    }
}
