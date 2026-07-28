use crate::info;
use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

pub struct UploadFailedHandler;
impl MessageHandler<PeerMessage> for UploadFailedHandler {
    fn get_code(&self) -> u8 {
        46
    }
    fn handle(&self, message: &mut Message, _sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        info!("Upload failed for {}", filename);
    }
}
