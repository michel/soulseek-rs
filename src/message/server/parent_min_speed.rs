use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    actor::server_actor::ServerOperation,
};

pub struct ParentMinSpeedHandler;

impl MessageHandler<ServerOperation> for ParentMinSpeedHandler {
    fn get_code(&self) -> u8 {
        83
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let _ = sender;
        let number = message.read_int32();
        debug!("Parent min speed: {}", number);
    }
}
