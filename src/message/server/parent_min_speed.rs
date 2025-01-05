use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    server::ServerOperation,
};

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
