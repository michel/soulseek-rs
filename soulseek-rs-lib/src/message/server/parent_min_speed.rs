use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct ParentMinSpeedHandler;

impl MessageHandler<ServerMessage> for ParentMinSpeedHandler {
    fn get_code(&self) -> u8 {
        83
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let _ = sender;
        let number = message.read_int32();
        debug!("Parent min speed: {}", number);
    }
}
