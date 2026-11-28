use crate::info;
use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

pub struct UploadDeniedHandler;
impl MessageHandler<PeerMessage> for UploadDeniedHandler {
    fn get_code(&self) -> u32 {
        50
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        let reason = message.read_string();
        info!("Upload denied for {}: {}", filename, reason);
        let reason = reason.trim();
        if reason == "Queued" {
            return;
        }
        let _ = sender.send(PeerMessage::UploadFailed {
            filename,
            reason: (!reason.is_empty()).then(|| reason.to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_queued_reason_is_not_a_failure() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\gone.mp3").write_string("Queued");
        });

        UploadDeniedHandler.handle(&mut message, tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_denied_filename_and_reason_are_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\gone.mp3")
                .write_string("File not shared.");
        });

        UploadDeniedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::UploadFailed { filename, reason }) => {
                assert_eq!(filename, "@@share\\gone.mp3");
                assert_eq!(reason.as_deref(), Some("File not shared."));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
