use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
    trace,
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
        let connection_type = message.read_string();
        let token = message.read_int32();
        trace!(
            "PeerInit: username: {}, connection_type: {}, token: {}",
            username,
            connection_type,
            token
        );

        sender.send(PeerOperation::SetUsername(username)).unwrap();
    }
}
