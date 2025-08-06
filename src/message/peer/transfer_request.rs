use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
    trace,
    types::Transfer,
};
use std::sync::mpsc::Sender;

pub struct TransferRequest;
impl MessageHandler<PeerOperation> for TransferRequest {
    fn get_code(&self) -> u8 {
        40
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        trace!("transfer_response: {:?}", message.get_buffer());
        let transfer = Transfer::new_from_message(message);

        sender
            .send(PeerOperation::TransferRequest(transfer))
            .unwrap();
    }
}
