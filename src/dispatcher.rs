use std::sync::mpsc::Sender;

use crate::{
    message::{handlers::Handlers, message::Message},
    server::ServerOperation,
};

pub struct MessageDispatcher {
    sender: Sender<ServerOperation>,
    handlers: Handlers,
}

impl MessageDispatcher {
    pub fn new(sender: Sender<ServerOperation>, handlers: Handlers) -> Self {
        let dispatcher = Self { handlers, sender };
        dispatcher
    }

    pub fn dispatch(&self, message: &mut Message) {
        let code = message.get_message_code();
        // println!("message with code: {}", code);
        match self.handlers.get_handler(code.clone()) {
            Some(handler) => {
                message.set_pointer(8);
                handler.handle(message, self.sender.clone());
            }
            None => {
                println!("No handler found for message code: {:?}", code);
            }
        }
    }
}
