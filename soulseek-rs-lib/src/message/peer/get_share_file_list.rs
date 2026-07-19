use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

/// A peer asking to browse our shared files (peer code 4). The client (which
/// owns the shares) builds the real SharedFileListResponse in reply.
pub struct GetShareFileList;
impl MessageHandler<PeerMessage> for GetShareFileList {
    fn get_code(&self) -> u8 {
        4
    }
    fn handle(&self, _message: &mut Message, sender: Sender<PeerMessage>) {
        let _ = sender.send(PeerMessage::ShareListRequested);
    }
}
