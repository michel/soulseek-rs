use std::sync::{mpsc::Sender, Arc, Condvar, Mutex};

use crate::{message::Message, server::ServerOperation};

use super::handlers::MessageHandler;
pub struct PrivilegedUsersHandler;

impl MessageHandler for PrivilegedUsersHandler {
    fn get_code(&self) -> u8 {
        69
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let number = message.read_int32();
        println!("Number of privilaged users: {}", number);
    }
}
