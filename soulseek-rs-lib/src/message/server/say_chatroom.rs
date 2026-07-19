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

    #[test]
    fn forwards_room_message() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = Message::new();
        message.write_raw_bytes(vec![0u8; 8]);
        message.write_string("jazz");
        message.write_string("alice");
        message.write_string("hello everyone");
        message.set_pointer(8);

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
