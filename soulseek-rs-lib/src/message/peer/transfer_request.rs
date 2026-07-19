use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
    types::Transfer,
};

pub struct TransferRequest;
impl MessageHandler<PeerMessage> for TransferRequest {
    fn get_code(&self) -> u8 {
        40
    }
    fn handle(&self, message: &mut Message, out: &mut Vec<PeerMessage>) {
        let transfer = Transfer::new_from_message(message);

        out.push(PeerMessage::TransferRequest(transfer));
    }
}
