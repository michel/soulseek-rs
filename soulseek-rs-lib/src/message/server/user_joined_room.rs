use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct UserJoinedRoomHandler;

impl MessageHandler<ServerMessage> for UserJoinedRoomHandler {
    fn get_code(&self) -> u8 {
        16
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        // UserJoinedRoom (code 16): room, username, then that user's stats,
        // which we don't need. Reading the first two fields is enough.
        let room = message.read_string();
        let username = message.read_string();
        let _ = sender.send(ServerMessage::RoomUserJoined { room, username });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_user_joined() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = Message::new();
        message.write_raw_bytes(vec![0u8; 8]);
        message.write_string("jazz");
        message.write_string("carol");
        message.set_pointer(8);

        UserJoinedRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomUserJoined { room, username }) => {
                assert_eq!(room, "jazz");
                assert_eq!(username, "carol");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
