use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerOperation,
    message::{Message, MessageHandler},
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
