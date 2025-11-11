use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

pub struct PlaceInQueueResponse;

impl MessageHandler<PeerMessage> for PlaceInQueueResponse {
    fn get_code(&self) -> u8 {
        43
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        let place = message.read_int32();

        sender
            .send(PeerMessage::PlaceInQueueResponse { filename, place })
            .unwrap();
    }
}
