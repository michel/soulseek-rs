//! `WatchUser` (code 5): subscribe to a user's status and share statistics.
//!
//! Watching is how a client keeps a buddy list current. After a `WatchUser`
//! request the server answers on the same code with the user's current
//! presence and stats, and from then on pushes every later change as an
//! ordinary `GetUserStatus` (code 7). `UnwatchUser` (code 6) cancels the
//! subscription and has no reply.
//!
//! Note on codes: the protocol has no `AddUserList`/`WatchedUserAdded`
//! /`WatchedUserRemoved` messages — 141/142/143 are `EnableRoomInvitations`,
//! `ChangePassword` and `AddRoomOperator`. Watching is 5/6, per the reference
//! protocol documentation.

use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

/// Receives the `WatchUser` reply (code 5): `username`, `exists`, and — when
/// the user exists — status plus the same share statistics `GetUserStats`
/// carries.
pub struct WatchUserHandler;

impl MessageHandler<ServerMessage> for WatchUserHandler {
    fn get_code(&self) -> u8 {
        5
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let username = message.read_string();
        let exists = message.read_bool();
        let watched = if exists {
            let status = message.read_int32();
            let average_speed = message.read_int32();
            let _upload_number = message.read_int64(); // obsolete
            let shared_files = message.read_int32();
            let shared_folders = message.read_int32();
            ServerMessage::WatchedUserReceived {
                username,
                exists,
                status: Some(status),
                average_speed: Some(average_speed),
                shared_files: Some(shared_files),
                shared_folders: Some(shared_folders),
            }
        } else {
            // An unknown username: the server stops after the flag, so there
            // is nothing further to read and nothing to record.
            ServerMessage::WatchedUserReceived {
                username,
                exists,
                status: None,
                average_speed: None,
                shared_files: None,
                shared_folders: None,
            }
        };
        let _ = sender.send(watched);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn existing_user_carries_status_and_stats() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("alice")
                .write_int8(1) // exists
                .write_int32(2) // status: online
                .write_int32(1024) // average speed
                .write_int64(0) // obsolete upload number
                .write_int32(4321) // shared files
                .write_int32(77); // shared folders
        });

        WatchUserHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::WatchedUserReceived {
                username,
                exists,
                status,
                average_speed,
                shared_files,
                shared_folders,
            }) => {
                assert_eq!(username, "alice");
                assert!(exists);
                assert_eq!(status, Some(2));
                assert_eq!(average_speed, Some(1024));
                assert_eq!(shared_files, Some(4321));
                assert_eq!(shared_folders, Some(77));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unknown_user_reports_absence_without_stats() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("ghost").write_int8(0);
        });

        WatchUserHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::WatchedUserReceived {
                username,
                exists,
                status,
                shared_files,
                ..
            }) => {
                assert_eq!(username, "ghost");
                assert!(!exists);
                assert_eq!(status, None);
                assert_eq!(shared_files, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn truncated_payload_does_not_panic() {
        // exists = 1 but the stats were cut off: the bounds-checked readers
        // must yield zeros rather than panicking on a hostile frame.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("alice").write_int8(1).write_int32(2);
        });

        WatchUserHandler.handle(&mut message, tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::WatchedUserReceived { .. })
        ));
    }
}
