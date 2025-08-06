use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
};
use std::sync::mpsc::Sender;

pub struct TransferResponse;

impl MessageHandler<PeerOperation> for TransferResponse {
    fn get_code(&self) -> u8 {
        41
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        let token = message.read_int32();
        let allowed = message.read_int8();

        let reason = if allowed == 0 {
            Some(message.read_string())
        } else {
            None
        };

        sender
            .send(PeerOperation::TransferResponse {
                token,
                allowed: allowed == 1,
                reason,
            })
            .unwrap();
    }
}

