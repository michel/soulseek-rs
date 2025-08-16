use crate::debug;
use crate::message::{Message, MessageHandler};
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;

pub struct GetUserStatsHandler;

impl MessageHandler<ServerOperation> for GetUserStatsHandler {
    fn get_code(&self) -> u32 {
        36
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let username = message.read_string();
        let avg_speed = message.read_int32();
        let download_num = message.read_int32();
        let something = message.read_int32(); // Unknown field
        let files = message.read_int32();
        let folders = message.read_int32();

        debug!(
            "GetUserStats - User: {}, AvgSpeed: {}, Downloads: {}, Files: {}, Folders: {}, Unknown: {}",
            username, avg_speed, download_num, files, folders, something
        );
    }
}
