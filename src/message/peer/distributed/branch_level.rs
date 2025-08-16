use crate::client::ClientOperation;
use crate::message::{Message, MessageHandler};
use crate::debug;
use std::sync::mpsc::Sender;

pub struct BranchLevel;

impl MessageHandler<ClientOperation> for BranchLevel {
    fn get_code(&self) -> u32 {
        4
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ClientOperation>) {
        message.set_pointer(4);
        let code = message.read_int8();
        if code != 4 {
            debug!("Expected BranchLevel code 4, got {}", code);
            return;
        }

        let level = message.read_int32();
        debug!("Received BranchLevel message: level={}", level);
    }
}