use crate::{debug, error, info, trace, warn};
use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
    types::UploadFailed,
};
use std::sync::mpsc::Sender;

pub struct UploadFailedHandler;
impl MessageHandler<PeerOperation> for UploadFailedHandler {
    fn get_code(&self) -> u8 {
        46
    }
    fn handle(&self, message: &mut Message, _sender: Sender<PeerOperation>) {
        let upload_failed = UploadFailed::new_from_message(message);
        info!("Upload failed for ${}", upload_failed.filename);
    }
}
