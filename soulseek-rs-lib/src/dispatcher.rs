use crate::message::{handlers::Handlers, Message};
use std::sync::mpsc::Sender;

use crate::warn;

pub struct MessageDispatcher<Op> {
    owner_name: String,
    sender: Sender<Op>,
    handlers: Handlers<Op>,
}

impl<Op> MessageDispatcher<Op> {
    pub fn new(
        owner_name: String,
        sender: Sender<Op>,
        handlers: Handlers<Op>,
    ) -> Self {
        MessageDispatcher {
            owner_name,
            sender,
            handlers,
        }
    }

    pub fn dispatch(&self, message: &mut Message) {
        let code = message.get_message_code();

        if let Some(handler) = self.handlers.get_handler(code) {
            message.set_pointer(8);
            handler.handle(message, self.sender.clone());
        } else {
            warn!(
                "[{}:dispatcher] No handler found for message code: {}",
                self.owner_name,
                message.get_message_code()
            );
        }
    }
}
