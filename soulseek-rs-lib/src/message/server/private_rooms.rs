//! Private rooms: who may enter one and who runs it.
//!
//! The server pushes the membership and operator rosters of every private
//! room we belong to right after login (codes 133 and 148), then narrates
//! changes to them: a member added or removed (134/135), our own membership
//! or operatorship granted or revoked (139/140, 145/146), and a room we asked
//! for that could not be created (1003).

use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

/// Read a `[room][count][username]...` roster, stopping at the payload's end.
fn read_roster(message: &mut Message) -> (String, Vec<String>) {
    let room = message.read_string();
    let count = message.read_int32();
    let mut users = Vec::new();
    for _ in 0..count {
        let username = message.read_string();
        if username.is_empty() {
            break;
        }
        users.push(username);
    }
    (room, users)
}

/// `RoomMembers` (code 133): who may enter a private room.
pub struct RoomMembersHandler;

impl MessageHandler<ServerMessage> for RoomMembersHandler {
    fn get_code(&self) -> u32 {
        133
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let (room, users) = read_roster(message);
        let _ = sender.send(ServerMessage::PrivateRoomMembers { room, users });
    }
}

/// `RoomOperators` (code 148): who runs a private room.
pub struct RoomOperatorsHandler;

impl MessageHandler<ServerMessage> for RoomOperatorsHandler {
    fn get_code(&self) -> u32 {
        148
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let (room, users) = read_roster(message);
        let _ =
            sender.send(ServerMessage::PrivateRoomOperators { room, users });
    }
}

/// One user added to (`added`) or removed from a private room's roster:
/// members (134/135) or operators (143/144).
pub struct RoomRosterChangeHandler {
    code: u32,
    members: bool,
    added: bool,
}

impl RoomRosterChangeHandler {
    #[must_use]
    pub const fn member_added() -> Self {
        Self {
            code: 134,
            members: true,
            added: true,
        }
    }

    #[must_use]
    pub const fn member_removed() -> Self {
        Self {
            code: 135,
            members: true,
            added: false,
        }
    }

    #[must_use]
    pub const fn operator_added() -> Self {
        Self {
            code: 143,
            members: false,
            added: true,
        }
    }

    #[must_use]
    pub const fn operator_removed() -> Self {
        Self {
            code: 144,
            members: false,
            added: false,
        }
    }
}

impl MessageHandler<ServerMessage> for RoomRosterChangeHandler {
    fn get_code(&self) -> u32 {
        self.code
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let username = message.read_string();
        let _ = sender.send(ServerMessage::PrivateRoomRosterChanged {
            room,
            username,
            members: self.members,
            added: self.added,
        });
    }
}

/// Our own standing in a private room changed: membership granted (139) or
/// revoked (140), operatorship granted (145) or revoked (146).
pub struct OwnRoomStandingHandler {
    code: u32,
    members: bool,
    granted: bool,
}

impl OwnRoomStandingHandler {
    #[must_use]
    pub const fn membership_granted() -> Self {
        Self {
            code: 139,
            members: true,
            granted: true,
        }
    }

    #[must_use]
    pub const fn membership_revoked() -> Self {
        Self {
            code: 140,
            members: true,
            granted: false,
        }
    }

    #[must_use]
    pub const fn operatorship_granted() -> Self {
        Self {
            code: 145,
            members: false,
            granted: true,
        }
    }

    #[must_use]
    pub const fn operatorship_revoked() -> Self {
        Self {
            code: 146,
            members: false,
            granted: false,
        }
    }
}

impl MessageHandler<ServerMessage> for OwnRoomStandingHandler {
    fn get_code(&self) -> u32 {
        self.code
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let _ = sender.send(ServerMessage::OwnRoomStandingChanged {
            room,
            members: self.members,
            granted: self.granted,
        });
    }
}

/// `CantCreateRoom` (code 1003): the room we asked to join could not be
/// created — the name is taken by a private room we are not a member of, or
/// it is otherwise refused.
pub struct CantCreateRoomHandler;

impl MessageHandler<ServerMessage> for CantCreateRoomHandler {
    fn get_code(&self) -> u32 {
        1003
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let _ = sender.send(ServerMessage::CantCreateRoom { room });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_member_roster_lists_the_rooms_members() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("club");
            m.write_int32(2);
            m.write_string("alice");
            m.write_string("bob");
        });

        RoomMembersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::PrivateRoomMembers { room, users }) => {
                assert_eq!(room, "club");
                assert_eq!(users, ["alice", "bob"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_roster_count_beyond_the_payload_stops_at_the_data() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("club");
            m.write_int32(u32::MAX);
            m.write_string("alice");
        });

        RoomOperatorsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::PrivateRoomOperators { users, .. }) => {
                assert_eq!(users, ["alice"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn each_roster_change_reports_which_roster_and_which_way() {
        for (handler, members, added) in [
            (RoomRosterChangeHandler::member_added(), true, true),
            (RoomRosterChangeHandler::member_removed(), true, false),
            (RoomRosterChangeHandler::operator_added(), false, true),
            (RoomRosterChangeHandler::operator_removed(), false, false),
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut message = framed(|m| {
                m.write_string("club");
                m.write_string("alice");
            });

            handler.handle(&mut message, tx);
            match rx.try_recv() {
                Ok(ServerMessage::PrivateRoomRosterChanged {
                    room,
                    username,
                    members: got_members,
                    added: got_added,
                }) => {
                    assert_eq!(room, "club");
                    assert_eq!(username, "alice");
                    assert_eq!((got_members, got_added), (members, added));
                }
                other => panic!("unexpected: {other:?}"),
            }
        }
    }

    #[test]
    fn our_own_standing_reports_the_room_and_the_change() {
        for (handler, members, granted) in [
            (OwnRoomStandingHandler::membership_granted(), true, true),
            (OwnRoomStandingHandler::membership_revoked(), true, false),
            (OwnRoomStandingHandler::operatorship_granted(), false, true),
            (OwnRoomStandingHandler::operatorship_revoked(), false, false),
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut message = framed(|m| {
                m.write_string("club");
            });

            handler.handle(&mut message, tx);
            match rx.try_recv() {
                Ok(ServerMessage::OwnRoomStandingChanged {
                    room,
                    members: got_members,
                    granted: got_granted,
                }) => {
                    assert_eq!(room, "club");
                    assert_eq!((got_members, got_granted), (members, granted));
                }
                other => panic!("unexpected: {other:?}"),
            }
        }
    }

    #[test]
    fn a_refused_room_names_itself() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("taken");
        });

        CantCreateRoomHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::CantCreateRoom { room }) => {
                assert_eq!(room, "taken");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
