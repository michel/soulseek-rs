use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

pub struct TransferResponse;

impl MessageHandler<PeerMessage> for TransferResponse {
    fn get_code(&self) -> u8 {
        41
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let token = message.read_int32();
        let allowed = message.read_int8();
        let reason = (allowed == 0).then(|| message.read_string());

        sender
            .send(PeerMessage::TransferResponse {
                token,
                allowed: allowed == 1,
                reason,
            })
            .unwrap();
    }
}
