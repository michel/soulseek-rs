use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
    types::{RoomUserStats, UserStatus},
};
use std::sync::mpsc::Sender;

pub struct JoinRoomHandler;

impl MessageHandler<ServerMessage> for JoinRoomHandler {
    fn get_code(&self) -> u8 {
        14
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        // JoinRoom (code 14): room name, the member usernames, then the
        // parallel per-member stat vectors.
        let room = message.read_string();
        let users = read_strings(message);
        let stats = parse_member_stats(message, &users);
        let _ = sender.send(ServerMessage::RoomJoined {
            room: room.clone(),
            users,
        });
        if !stats.is_empty() {
            let _ = sender.send(ServerMessage::RoomMemberStats { room, stats });
        }
    }
}

/// Read a length-prefixed vector of strings, stopping early if the count
/// outruns the payload: a bogus (possibly hostile) count must not spin us
/// into an OOM allocation loop. A 4-byte floor is the smallest a
/// length-prefixed string can be.
fn read_strings(message: &mut Message) -> Vec<String> {
    let count = message.read_int32();
    let mut values = Vec::new();
    for _ in 0..count {
        if message.get_pointer() + 4 > message.get_size() {
            break;
        }
        values.push(message.read_string());
    }
    values
}

/// Read a length-prefixed vector of 32-bit values, bounded the same way.
fn read_int32s(message: &mut Message) -> Vec<u32> {
    let count = message.read_int32();
    let mut values = Vec::new();
    for _ in 0..count {
        if message.get_pointer() + 4 > message.get_size() {
            break;
        }
        values.push(message.read_int32());
    }
    values
}

/// Parse the per-member statistics that follow the membership list of a
/// `JoinRoom` (code 14) reply: statuses, then a record of share statistics per
/// member, then free upload slots, then country codes.
///
/// Each vector is independently length-prefixed and a server may send fewer
/// entries than there are members (or, on an older server, omit the country
/// vector entirely), so every field is looked up positionally and defaults
/// when absent rather than shifting the whole roster.
#[must_use]
fn parse_member_stats(
    message: &mut Message,
    users: &[String],
) -> Vec<RoomUserStats> {
    let statuses = read_int32s(message);

    let stat_count = message.read_int32();
    let mut shares = Vec::new();
    for _ in 0..stat_count {
        // A share record is 20 bytes: speed, an obsolete 64-bit upload
        // number, files and folders.
        if message.get_pointer() + 20 > message.get_size() {
            break;
        }
        let average_speed = message.read_int32();
        let _upload_number = message.read_int64(); // obsolete
        let shared_files = message.read_int32();
        let shared_folders = message.read_int32();
        shares.push((average_speed, shared_files, shared_folders));
    }

    let slots_free = read_int32s(message);
    let countries = read_strings(message);

    users
        .iter()
        .enumerate()
        .map(|(i, username)| {
            let (average_speed, shared_files, shared_folders) =
                shares.get(i).copied().unwrap_or((0, 0, 0));
            RoomUserStats {
                username: username.clone(),
                status: UserStatus::from_code(
                    statuses.get(i).copied().unwrap_or(0),
                ),
                average_speed,
                shared_files,
                shared_folders,
                slots_free: slots_free.get(i).copied().unwrap_or(0),
                country: countries.get(i).cloned().filter(|c| !c.is_empty()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn hostile_user_count_does_not_hang() {
        // room="" then user_count=u32::MAX with no usernames: must return
        // promptly instead of looping ~4 billion times.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("").write_int32(u32::MAX);
        });

        JoinRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomJoined { users, .. }) => {
                assert!(users.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// A full JoinRoom payload: room, members, then the four parallel stat
    /// vectors the server sends for them.
    fn join_payload() -> Message {
        framed(|m| {
            m.write_string("nicotine")
                .write_int32(2)
                .write_string("alice")
                .write_string("bob")
                // statuses
                .write_int32(2)
                .write_int32(2)
                .write_int32(1)
                // share records
                .write_int32(2)
                .write_int32(1024)
                .write_int64(0)
                .write_int32(4321)
                .write_int32(77)
                .write_int32(50)
                .write_int64(0)
                .write_int32(10)
                .write_int32(2)
                // free slots
                .write_int32(2)
                .write_int32(1)
                .write_int32(0)
                // countries
                .write_int32(2)
                .write_string("NL")
                .write_string("");
        })
    }

    #[test]
    fn member_stats_are_parsed_alongside_the_roster() {
        let (tx, rx) = std::sync::mpsc::channel();
        JoinRoomHandler.handle(&mut join_payload(), tx);

        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::RoomJoined { .. })
        ));
        match rx.try_recv() {
            Ok(ServerMessage::RoomMemberStats { room, stats }) => {
                assert_eq!(room, "nicotine");
                assert_eq!(
                    stats,
                    vec![
                        RoomUserStats {
                            username: "alice".to_string(),
                            status: UserStatus::Online,
                            average_speed: 1024,
                            shared_files: 4321,
                            shared_folders: 77,
                            slots_free: 1,
                            country: Some("NL".to_string()),
                        },
                        RoomUserStats {
                            username: "bob".to_string(),
                            status: UserStatus::Away,
                            average_speed: 50,
                            shared_files: 10,
                            shared_folders: 2,
                            slots_free: 0,
                            country: None,
                        },
                    ]
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_server_that_omits_the_stat_vectors_still_yields_a_roster() {
        // Nothing after the membership list: every stat reads as a default
        // rather than shifting the roster or dropping members.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz").write_int32(1).write_string("alice");
        });

        JoinRoomHandler.handle(&mut message, tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::RoomJoined { .. })
        ));
        match rx.try_recv() {
            Ok(ServerMessage::RoomMemberStats { stats, .. }) => {
                assert_eq!(stats.len(), 1);
                assert_eq!(stats[0].username, "alice");
                assert_eq!(stats[0].average_speed, 0);
                assert_eq!(stats[0].country, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn short_stat_vectors_do_not_shift_the_roster() {
        // Stats for the first member only: the second must still be reported,
        // with defaults, rather than inheriting the first member's figures.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz")
                .write_int32(2)
                .write_string("alice")
                .write_string("bob")
                .write_int32(1)
                .write_int32(2)
                .write_int32(1)
                .write_int32(1024)
                .write_int64(0)
                .write_int32(9)
                .write_int32(1)
                .write_int32(0)
                .write_int32(0);
        });

        JoinRoomHandler.handle(&mut message, tx);
        let _ = rx.try_recv();
        match rx.try_recv() {
            Ok(ServerMessage::RoomMemberStats { stats, .. }) => {
                assert_eq!(stats.len(), 2);
                assert_eq!(stats[0].shared_files, 9);
                assert_eq!(stats[1].username, "bob");
                assert_eq!(stats[1].shared_files, 0);
                assert_eq!(stats[1].status, UserStatus::Offline);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn hostile_stat_counts_do_not_hang_or_overallocate() {
        // Vectors claiming ~4 billion entries in a tiny frame must return
        // promptly, bounded by the payload.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz")
                .write_int32(1)
                .write_string("alice")
                .write_int32(u32::MAX);
        });

        JoinRoomHandler.handle(&mut message, tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::RoomJoined { .. })
        ));
    }

    #[test]
    fn forwards_room_and_member_list() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("nicotine")
                .write_int32(2)
                .write_string("alice")
                .write_string("bob");
        });

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
