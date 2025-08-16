use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
    types::Transfer,
};
use std::sync::mpsc::Sender;

pub struct TransferRequest;
impl MessageHandler<PeerOperation> for TransferRequest {
    fn get_code(&self) -> u32 {
        40
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        let transfer = Transfer::new_from_message(message);

        sender
            .send(PeerOperation::TransferRequest(transfer))
            .unwrap();
    }
}
