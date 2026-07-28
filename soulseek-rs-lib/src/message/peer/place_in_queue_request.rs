use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

/// `PlaceInQueueRequest` (peer code 51): a peer asking where the file they
/// queued sits in our upload queue.
///
/// Worth answering rather than dropping: a downloader that gets no position
/// cannot tell a long queue from a client that has forgotten about it.
pub struct PlaceInQueueRequest;

impl MessageHandler<PeerMessage> for PlaceInQueueRequest {
    fn get_code(&self) -> u8 {
        51
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        let _ = sender.send(PeerMessage::IncomingPlaceInQueueRequest(filename));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_asked_about_filename_is_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\track.mp3");
        });

        PlaceInQueueRequest.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::IncomingPlaceInQueueRequest(filename)) => {
                assert_eq!(filename, "@@share\\track.mp3");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
