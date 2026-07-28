use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct UserLeftRoomHandler;

impl MessageHandler<ServerMessage> for UserLeftRoomHandler {
    fn get_code(&self) -> u8 {
        17
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let username = message.read_string();
        let _ = sender.send(ServerMessage::RoomUserLeft { room, username });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn forwards_user_left() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz").write_string("carol");
        });

        UserLeftRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomUserLeft { room, username }) => {
                assert_eq!(room, "jazz");
                assert_eq!(username, "carol");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
