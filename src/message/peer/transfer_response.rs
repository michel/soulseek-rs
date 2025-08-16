use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
};
use std::sync::mpsc::Sender;

pub struct TransferResponse;

impl MessageHandler<PeerOperation> for TransferResponse {
    fn get_code(&self) -> u32 {
        41
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        let token = message.read_raw_bytes(4);
        let allowed = message.read_int8();

        let reason = if allowed == 0 {
            // When not allowed, read the reason string
            Some(message.read_string())
        } else {
            // When allowed, no additional data
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
