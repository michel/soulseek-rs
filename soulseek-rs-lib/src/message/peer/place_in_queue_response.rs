use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};

pub struct PlaceInQueueResponse;

impl MessageHandler<PeerMessage> for PlaceInQueueResponse {
    fn get_code(&self) -> u8 {
        43
    }

    fn handle(&self, message: &mut Message, out: &mut Vec<PeerMessage>) {
        let filename = message.read_string();
        let place = message.read_int32();

        out.push(PeerMessage::PlaceInQueueResponse { filename, place });
    }
}
