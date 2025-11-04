use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
};

pub struct PeerInit;
impl MessageHandler<PeerOperation> for PeerInit {
    fn get_code(&self) -> u8 {
        1
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        message.set_pointer(4);
        let _message_code = message.read_int8();
        let username = message.read_string();
        sender.send(PeerOperation::SetUsername(username)).unwrap();
    }
}
