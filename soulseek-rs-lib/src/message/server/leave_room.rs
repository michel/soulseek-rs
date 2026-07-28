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
    use crate::message::framed;

    #[test]
    fn forwards_left_room() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
        });

        LeaveRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomLeft { room }) => assert_eq!(room, "jazz"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
