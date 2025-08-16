use crate::client::ClientOperation;
use crate::debug;
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

pub struct BranchRoot;

impl MessageHandler<ClientOperation> for BranchRoot {
    fn get_code(&self) -> u32 {
        5
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ClientOperation>) {
        message.set_pointer(4);
        let code = message.read_int8();
        if code != 5 {
            debug!("Expected BranchRoot code 5, got {}", code);
            return;
        }

        let root = message.read_string();
        debug!("Received BranchRoot message: root={}", root);
    }
}
