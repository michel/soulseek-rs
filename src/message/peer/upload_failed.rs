
use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
    types::UploadFailed,
};
use std::sync::mpsc::Sender;

pub struct UploadFailedHandler;
impl MessageHandler<PeerOperation> for UploadFailedHandler {
    fn get_code(&self) -> u32 {
        46
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        let upload_failed = UploadFailed::new_from_message(message);
        
        sender
            .send(PeerOperation::UploadFailed {
                filename: upload_failed.filename,
            })
            .unwrap();
    }
}
