use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};

/// A peer asking to download one of our shared files (peer code 43).
pub struct QueueUploadHandler;

impl MessageHandler<PeerMessage> for QueueUploadHandler {
    fn get_code(&self) -> u8 {
        43
    }

    fn handle(&self, message: &mut Message, out: &mut Vec<PeerMessage>) {
        let filename = message.read_string();
        out.push(PeerMessage::IncomingQueueUpload(filename));
    }
}
