use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct LeaveRoomHandler;

impl MessageHandler<ServerMessage> for LeaveRoomHandler {
    fn get_code(&self) -> u8 {
        15
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let _ = sender.send(ServerMessage::RoomLeft { room });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_left_room() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = Message::new();
        message.write_raw_bytes(vec![0u8; 8]);
        message.write_string("jazz");
        message.set_pointer(8);

        LeaveRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomLeft { room }) => assert_eq!(room, "jazz"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
