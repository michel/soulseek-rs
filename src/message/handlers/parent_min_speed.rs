use std::sync::{mpsc::Sender, Arc, Condvar, Mutex};

use crate::{message::Message, server::ServerOperation};

use super::handlers::MessageHandler;
pub struct ParentMinSpeedHandler;

impl MessageHandler for ParentMinSpeedHandler {
    fn get_code(&self) -> u8 {
        83
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let _ = sender;
        let number = message.read_int32();
        println!("Parent min speed: {}", number);
    }
}
