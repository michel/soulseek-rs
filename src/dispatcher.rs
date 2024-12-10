use crate::{
    message::{handlers::handlers::Handlers, Message},
    server::{Context, ServerOperation},
};
use std::sync::{mpsc::Sender, Arc, Mutex};

pub struct MessageDispatcher {
    sender: Sender<ServerOperation>,
    handlers: Handlers,
}

impl MessageDispatcher {
    pub fn new(sender: Sender<ServerOperation>) -> Self {
        let handlers = Handlers::new_with_default_handlers();
        let dispatcher = Self { handlers, sender };
        dispatcher
    }

    pub fn dispatch(&self, message: &mut Message) {
        match self.handlers.get_handler(message.get_message_code()) {
            Some(handler) => {
                message.set_pointer(8);
                handler.handle(message, self.sender.clone());
            }
            None => {
                println!(
                    "No handler found for message code: {}",
                    message.get_message_code()
                );
            }
        }
    }
}
