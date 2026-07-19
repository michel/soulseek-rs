use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct JoinRoomHandler;

impl MessageHandler<ServerMessage> for JoinRoomHandler {
    fn get_code(&self) -> u8 {
        14
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        // JoinRoom (code 14): room name, then a vector of member usernames.
        // Per-user stat vectors follow but are not needed here, so we stop
        // after reading the names.
        let room = message.read_string();
        let user_count = message.read_int32();
        let mut users = Vec::new();
        for _ in 0..user_count {
            users.push(message.read_string());
        }
        let _ = sender.send(ServerMessage::RoomJoined { room, users });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_room_and_member_list() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = Message::new();
        message.write_raw_bytes(vec![0u8; 8]);
        message.write_string("nicotine");
        message.write_int32(2);
        message.write_string("alice");
        message.write_string("bob");
        message.set_pointer(8);

        JoinRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomJoined { room, users }) => {
                assert_eq!(room, "nicotine");
                assert_eq!(users, vec!["alice", "bob"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
