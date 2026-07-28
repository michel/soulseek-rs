use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct SayChatroomHandler;

impl MessageHandler<ServerMessage> for SayChatroomHandler {
    fn get_code(&self) -> u8 {
        13
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let username = message.read_string();
        let message_text = message.read_string();
        let _ = sender.send(ServerMessage::RoomMessageReceived {
            room,
            username,
            message: message_text,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn forwards_room_message() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz")
                .write_string("alice")
                .write_string("hello everyone");
        });

        SayChatroomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomMessageReceived {
                room,
                username,
                message,
            }) => {
                assert_eq!(room, "jazz");
                assert_eq!(username, "alice");
                assert_eq!(message, "hello everyone");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
