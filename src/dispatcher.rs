use crate::{
    message::{handlers::handlers::Handlers, Message},
    server::Context,
};
use std::sync::{Arc, Mutex};

pub struct MessageDispatcher {
    context: Arc<Mutex<Context>>,
    handlers: Handlers,
}

impl MessageDispatcher {
    pub fn new(context: Arc<Mutex<Context>>) -> Self {
        let handlers = Handlers::new_with_default_handlers();
        let dispatcher = Self { context, handlers };
        dispatcher
    }

    pub fn dispatch(&self, message: &mut Message) {
        match self.handlers.get_handler(message.get_message_code()) {
            Some(handler) => {
                message.set_pointer(8);
                handler.handle(message, self.context.clone());
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
