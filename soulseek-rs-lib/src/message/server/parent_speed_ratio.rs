use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct ParentSpeedRatioHandler;

// The server sends us a speed ratio determining the number of children we can have in the distributed network. The maximum number of children is our upload speed divided by the speed ratio.
impl MessageHandler<ServerMessage> for ParentSpeedRatioHandler {
    fn get_code(&self) -> u32 {
        84
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let number = message.read_int32();
        debug!("Parent speed ratio: {}", number);
        let _ = sender.send(ServerMessage::ParentSpeedRatio(number));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_ratio_is_reported_not_discarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(50);
        });

        ParentSpeedRatioHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::ParentSpeedRatio(ratio)) => {
                assert_eq!(ratio, 50);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
