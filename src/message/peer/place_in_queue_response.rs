use crate::{
    message::{Message, MessageHandler},
    peer::PeerOperation,
};
use std::sync::mpsc::Sender;

pub struct PlaceInQueueResponse;

impl MessageHandler<PeerOperation> for PlaceInQueueResponse {
    fn get_code(&self) -> u32 {
        44
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerOperation>) {
        let filename = message.read_string();
        let place = message.read_int32() as u32;

        sender
            .send(PeerOperation::PlaceInQueueResponse { filename, place })
            .unwrap();
    }
}