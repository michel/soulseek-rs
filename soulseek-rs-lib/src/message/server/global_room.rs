use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

/// `GlobalRoomMessage` (code 152): a message said in some public room,
/// delivered because we subscribed to the global feed (code 150).
pub struct GlobalRoomMessageHandler;

impl MessageHandler<ServerMessage> for GlobalRoomMessageHandler {
    fn get_code(&self) -> u32 {
        152
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let username = message.read_string();
        let text = message.read_string();
        let _ = sender.send(ServerMessage::GlobalRoomMessageReceived {
            room,
            username,
            message: text,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_global_message_names_the_room_it_was_said_in() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_string("alice");
            m.write_string("anyone here?");
        });

        GlobalRoomMessageHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::GlobalRoomMessageReceived {
                room,
                username,
                message,
            }) => {
                assert_eq!(
                    (room.as_str(), username.as_str(), message.as_str()),
                    ("jazz", "alice", "anyone here?")
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
