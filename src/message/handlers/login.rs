use std::sync::{Arc, Mutex};

use crate::{
    dispatcher::MessageHandler,
    message::{factory::build_shared_folders_message, Message},
    server::Context,
};
pub struct LoginHandler;

impl MessageHandler for LoginHandler {
    fn handle(&self, message: &mut Message, context: Arc<Mutex<Context>>) {
        let response = message.read_int8();
        if response == 1 {
            println!("Login successful");
            let greeting = message.read_string();
            println!("Server geeting: {:?}", greeting);
            // Build the shared folders message and queue it
            let shared_message = build_shared_folders_message(1, 1);
            context.lock().unwrap().queue_message(shared_message);
        } else {
            println!("Login failed");
        }
    }
}
