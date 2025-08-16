use crate::debug;
use crate::message::{Message, MessageHandler};
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;

pub struct CannotConnectToPeerHandler;

impl MessageHandler<ServerOperation> for CannotConnectToPeerHandler {
    fn get_code(&self) -> u32 {
        1001
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let token = message.read_raw_bytes(4);
        debug!("CannotConnectToPeer - Token: {:?}", token);
    }
}
