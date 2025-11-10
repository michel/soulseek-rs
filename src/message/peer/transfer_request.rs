use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
    types::Transfer,
};
use std::sync::mpsc::Sender;

pub struct TransferRequest;
impl MessageHandler<PeerMessage> for TransferRequest {
    fn get_code(&self) -> u8 {
        40
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let transfer = Transfer::new_from_message(message);

        sender.send(PeerMessage::TransferRequest(transfer)).unwrap();
    }
}
