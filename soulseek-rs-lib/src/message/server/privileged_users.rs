use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

/// `PrivilegedUsers` (code 69): everyone the server counts as having donated,
/// sent once at login.
///
/// The names matter, not the count: they decide who overtakes whom in our
/// upload queue, which is the "recognition of privileges" an alternative client
/// is expected to implement.
pub struct PrivilegedUsersHandler;

impl MessageHandler<ServerMessage> for PrivilegedUsersHandler {
    fn get_code(&self) -> u8 {
        69
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let count = message.read_int32();
        let mut users = Vec::new();
        for _ in 0..count {
            // Guard against a hostile count outrunning the payload.
            if message.get_pointer() + 4 > message.get_size() {
                break;
            }
            users.push(message.read_string());
        }
        let _ = sender.send(ServerMessage::PrivilegedUsers(users));
    }
}

/// `CheckPrivileges` (code 92): how many seconds of our own privileges are
/// left. Zero means we have none.
pub struct CheckPrivilegesHandler;

impl MessageHandler<ServerMessage> for CheckPrivilegesHandler {
    fn get_code(&self) -> u8 {
        92
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let seconds = message.read_int32();
        let _ = sender.send(ServerMessage::OwnPrivileges(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_user_list_is_kept_not_just_counted() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(2)
                .write_string("donor_a")
                .write_string("donor_b");
        });

        PrivilegedUsersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::PrivilegedUsers(users)) => {
                assert_eq!(users, ["donor_a", "donor_b"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn an_empty_list_is_still_an_answer() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(0);
        });

        PrivilegedUsersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::PrivilegedUsers(users)) => {
                assert!(users.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_hostile_count_does_not_hang() {
        // count=u32::MAX with no names: the guard must return promptly rather
        // than loop four billion times.
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(u32::MAX);
        });
        PrivilegedUsersHandler.handle(&mut message, tx);
    }

    #[test]
    fn our_own_remaining_privilege_time_is_reported() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(86_400);
        });

        CheckPrivilegesHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::OwnPrivileges(seconds)) => {
                assert_eq!(seconds, 86_400);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
