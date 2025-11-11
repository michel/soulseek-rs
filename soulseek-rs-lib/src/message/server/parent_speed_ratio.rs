use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct ParentSpeedRatioHandler;

// The server sends us a speed ratio determining the number of children we can have in the distributed network. The maximum number of children is our upload speed divided by the speed ratio.
impl MessageHandler<ServerMessage> for ParentSpeedRatioHandler {
    fn get_code(&self) -> u8 {
        84
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerMessage>) {
        let number = message.read_int32();
        debug!("Parent speed ratio: {}", number);
    }
}
