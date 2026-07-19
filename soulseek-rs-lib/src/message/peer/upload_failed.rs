use crate::info;
use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
    types::UploadFailed,
};

pub struct UploadFailedHandler;
impl MessageHandler<PeerMessage> for UploadFailedHandler {
    fn get_code(&self) -> u8 {
        46
    }
    fn handle(&self, message: &mut Message, _out: &mut Vec<PeerMessage>) {
        let upload_failed = UploadFailed::new_from_message(message);
        info!("Upload failed for ${}", upload_failed.filename);
    }
}
