use std::sync::mpsc::Sender;

use crate::{message::Message, server::ServerOperation};

use super::handlers::MessageHandler;
pub struct ParentSpeedRatioHandler;

// The server sends us a speed ratio determining the number of children we can have in the distributed network. The maximum number of children is our upload speed divided by the speed ratio.
impl MessageHandler for ParentSpeedRatioHandler {
    fn get_code(&self) -> u8 {
        84
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let number = message.read_int32();
        println!("Parent min speed: {}", number);
    }
}
