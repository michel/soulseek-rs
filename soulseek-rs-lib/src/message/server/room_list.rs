use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
    types::RoomInfo,
};
use std::sync::mpsc::Sender;

pub struct RoomListHandler;

impl MessageHandler<ServerMessage> for RoomListHandler {
    fn get_code(&self) -> u8 {
        64
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let rooms = parse_room_list(message);
        let _ = sender.send(ServerMessage::RoomListReceived(rooms));
    }
}

/// Parse the public rooms out of a `RoomList` (code 64) message: a vector of
/// room names followed by a vector of user counts. The remaining private-room
/// sections are ignored. `message` must be positioned at the payload (the
/// dispatcher sets pointer 8).
#[must_use]
pub fn parse_room_list(message: &mut Message) -> Vec<RoomInfo> {
    let name_count = message.read_int32();
    let mut names = Vec::new();
    for _ in 0..name_count {
        names.push(message.read_string());
    }
    let count_count = message.read_int32();
    let mut counts = Vec::new();
    for _ in 0..count_count {
        counts.push(message.read_int32());
    }
    names
        .into_iter()
        .zip(counts)
        .map(|(name, user_count)| RoomInfo { name, user_count })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a received message (8-byte header + payload) and position it the
    /// way the dispatcher would.
    fn framed(build: impl FnOnce(&mut Message)) -> Message {
        let mut message = Message::new();
        message.write_raw_bytes(vec![0u8; 8]);
        build(&mut message);
        message.set_pointer(8);
        message
    }

    #[test]
    fn parses_names_zipped_with_counts() {
        let mut message = framed(|m| {
            m.write_int32(2);
            m.write_string("nicotine");
            m.write_string("jazz");
            m.write_int32(2);
            m.write_int32(42);
            m.write_int32(7);
        });
        let rooms = parse_room_list(&mut message);
        assert_eq!(
            rooms,
            vec![
                RoomInfo {
                    name: "nicotine".to_string(),
                    user_count: 42
                },
                RoomInfo {
                    name: "jazz".to_string(),
                    user_count: 7
                },
            ]
        );
    }

    #[test]
    fn empty_room_list_parses_to_empty() {
        let mut message = framed(|m| {
            m.write_int32(0);
            m.write_int32(0);
        });
        assert!(parse_room_list(&mut message).is_empty());
    }

    #[test]
    fn handler_forwards_parsed_rooms() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(1);
            m.write_string("room");
            m.write_int32(1);
            m.write_int32(5);
        });
        RoomListHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomListReceived(rooms)) => {
                assert_eq!(rooms.len(), 1);
                assert_eq!(rooms[0].name, "room");
                assert_eq!(rooms[0].user_count, 5);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
