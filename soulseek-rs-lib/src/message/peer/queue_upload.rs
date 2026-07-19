use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

/// A peer asking to download one of our shared files (peer code 43).
pub struct QueueUploadHandler;

impl MessageHandler<PeerMessage> for QueueUploadHandler {
    fn get_code(&self) -> u8 {
        43
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        let _ = sender.send(PeerMessage::IncomingQueueUpload(filename));
    }
}
