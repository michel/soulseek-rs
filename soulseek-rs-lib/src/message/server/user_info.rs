//! `GetUserStatus` (code 7) and `GetUserStats` (code 36): what the server
//! knows about another user.
//!
//! The two answers arrive as separate messages, so a caller that wants a full
//! picture asks for both and merges them — see `Client::request_user_info`.

use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

/// Receives `GetUserStatus` (code 7): `username`, `status`, `privileged`.
pub struct GetUserStatusHandler;

impl MessageHandler<ServerMessage> for GetUserStatusHandler {
    fn get_code(&self) -> u8 {
        7
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let username = message.read_string();
        let status = message.read_int32();
        let privileged = message.read_bool();
        let _ = sender.send(ServerMessage::UserStatusReceived {
            username,
            status,
            privileged,
        });
    }
}

/// Receives `GetUserStats` (code 36): `username`, `average upload speed`, two
/// obsolete fields, then the shared file and folder counts.
pub struct GetUserStatsHandler;

impl MessageHandler<ServerMessage> for GetUserStatsHandler {
    fn get_code(&self) -> u8 {
        36
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let username = message.read_string();
        let average_speed = message.read_int32();
        let _upload_number = message.read_int32(); // obsolete, always 0
        let _unknown = message.read_int32(); // obsolete, always 0
        let shared_files = message.read_int32();
        let shared_folders = message.read_int32();
        let _ = sender.send(ServerMessage::UserStatsReceived {
            username,
            average_speed,
            shared_files,
            shared_folders,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn user_status_is_parsed_with_its_privileged_flag() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("alice").write_int32(2).write_int8(1);
        });

        GetUserStatusHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::UserStatusReceived {
                username,
                status,
                privileged,
            }) => {
                assert_eq!(username, "alice");
                assert_eq!(status, 2);
                assert!(privileged);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn user_stats_skip_the_two_obsolete_fields() {
        let (tx, rx) = std::sync::mpsc::channel();
        // username, speed, obsolete, obsolete, files, folders
        let mut message = framed(|m| {
            m.write_string("bob")
                .write_int32(1024)
                .write_int32(0)
                .write_int32(0)
                .write_int32(4321)
                .write_int32(77);
        });

        GetUserStatsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::UserStatsReceived {
                username,
                average_speed,
                shared_files,
                shared_folders,
            }) => {
                assert_eq!(username, "bob");
                assert_eq!(average_speed, 1024);
                assert_eq!(shared_files, 4321, "must skip the obsolete pair");
                assert_eq!(shared_folders, 77);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
