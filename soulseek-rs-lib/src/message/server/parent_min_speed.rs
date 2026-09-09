use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct ParentMinSpeedHandler;

impl MessageHandler<ServerMessage> for ParentMinSpeedHandler {
    fn get_code(&self) -> u32 {
        83
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let number = message.read_int32();
        debug!("Parent min speed: {}", number);
        // Kept, not just logged: with the ratio below it decides how many
        // children this client may carry.
        let _ = sender.send(ServerMessage::ParentMinSpeed(number));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_minimum_speed_is_reported_not_discarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(1024);
        });

        ParentMinSpeedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::ParentMinSpeed(speed)) => {
                assert_eq!(speed, 1024);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
