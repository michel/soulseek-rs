use crate::info;
use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

pub struct UploadFailedHandler;
impl MessageHandler<PeerMessage> for UploadFailedHandler {
    fn get_code(&self) -> u8 {
        46
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        info!("Upload failed for {}", filename);
        let _ = sender.send(PeerMessage::UploadFailed(String::new(), filename));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_failed_filename_is_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\gone.mp3");
        });

        UploadFailedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::UploadFailed(_, filename)) => {
                assert_eq!(filename, "@@share\\gone.mp3");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
