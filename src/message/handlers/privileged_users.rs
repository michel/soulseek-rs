use std::sync::{Arc, Mutex};

use crate::{dispatcher::MessageHandler, message::Message, server::Context};
pub struct PrivilegedUsersHandler;

impl MessageHandler for PrivilegedUsersHandler {
    fn handle(&self, message: &mut Message, _context: Arc<Mutex<Context>>) {
        let number = message.read_int32();
        println!("Number of privilaged users: {}", number);
    }
}
