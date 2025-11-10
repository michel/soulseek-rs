use crate::{
    message::{server::MessageFactory, Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

pub struct GetShareFileList;
impl MessageHandler<PeerMessage> for GetShareFileList {
    fn get_code(&self) -> u8 {
        4
    }
    fn handle(&self, _message: &mut Message, sender: Sender<PeerMessage>) {
        let message = MessageFactory::build_shared_folders_message(100, 800);

        sender.send(PeerMessage::SendMessage(message)).unwrap();
    }
}
