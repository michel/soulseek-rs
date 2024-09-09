use crate::{
    dispatcher::MessageHandler,
    message::{factory::build_shared_folders_message, Message},
};
pub struct LoginHandler;

impl MessageHandler for LoginHandler {
    fn handle(&self, message: &mut Message, context: crate::server::Context) {
        let response = message.read_int8();
        if response == 1 {
            println!("Login successful");
            let greeting = message.read_string();
            println!("Server geeting: {:?}", greeting);
            // Build the shared folders message and queue it
            let shared_message = build_shared_folders_message(1, 1).get_data();
            context.queue_message(shared_message);
        } else {
            println!("Login failed");
        }
    }
}
