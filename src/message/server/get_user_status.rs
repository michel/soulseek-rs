use crate::debug;
use crate::message::{Message, MessageHandler};
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;

pub struct GetUserStatusHandler;

impl MessageHandler<ServerOperation> for GetUserStatusHandler {
    fn get_code(&self) -> u32 {
        7
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let username = message.read_string();
        let status = message.read_int32();

        let status_text = match status {
            0 => "Offline",
            1 => "Away",
            2 => "Online",
            _ => "Unknown",
        };

        debug!(
            "GetUserStatus - User: {}, Status: {} ({})",
            username, status, status_text
        );
    }
}
