use crate::{
    message::{Message, MessageHandler, server::MessageFactory},
    peer::PeerMessage,
};

pub struct GetShareFileList;
impl MessageHandler<PeerMessage> for GetShareFileList {
    fn get_code(&self) -> u8 {
        4
    }
    fn handle(&self, _message: &mut Message, out: &mut Vec<PeerMessage>) {
        let message = MessageFactory::build_shared_folders_message(100, 800);

        out.push(PeerMessage::SendMessage(message));
    }
}
