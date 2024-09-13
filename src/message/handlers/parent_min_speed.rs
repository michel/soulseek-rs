use std::sync::{Arc, Mutex};

use crate::{message::Message, server::Context};

use super::handlers::MessageHandler;
pub struct ParentMinSpeedHandler;

impl MessageHandler for ParentMinSpeedHandler {
    fn get_code(&self) -> u8 {
        83
    }

    fn handle(&self, message: &mut Message, _context: Arc<Mutex<Context>>) {
        let number = message.read_int32();
        println!("Parent min speed: {}", number);
    }
}
