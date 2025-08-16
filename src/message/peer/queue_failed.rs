use crate::debug;
use crate::message::{Message, MessageHandler};
use crate::peer::PeerOperation;
use std::sync::mpsc::Sender;

pub struct QueueFailedHandler;

impl MessageHandler<PeerOperation> for QueueFailedHandler {
    fn get_code(&self) -> u32 {
        50
    }

    fn handle(&self, message: &mut Message, _sender: Sender<PeerOperation>) {
        let filename = message.read_string();
        let reason = message.read_string();

        debug!("QueueFailed for file: {} - Reason: {}", filename, reason);

        // TODO: Notify the user or handle the queue failure
    }
}
