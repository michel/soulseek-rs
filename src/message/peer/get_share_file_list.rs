use crate::{
    message::{server::MessageFactory, Message, MessageHandler},
    peer::PeerOperation,
};
use std::sync::mpsc::Sender;

pub struct GetShareFileList;
impl MessageHandler<PeerOperation> for GetShareFileList {
    fn get_code(&self) -> u32 {
        4
    }
    fn handle(&self, _message: &mut Message, sender: Sender<PeerOperation>) {
        let message = MessageFactory::build_shared_folders_message(100, 800);

        sender.send(PeerOperation::SendMessage(message)).unwrap();
    }
}
