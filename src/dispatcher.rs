use crate::message::{handlers::Handlers, Message};
use std::sync::mpsc::Sender;

/// Add `<Op>` to make it generic over any operation type.
pub struct MessageDispatcher<Op> {
    sender: Sender<Op>,
    handlers: Handlers<Op>,
}

impl<Op> MessageDispatcher<Op> {
    pub fn new(sender: Sender<Op>, handlers: Handlers<Op>) -> Self {
        MessageDispatcher { sender, handlers }
    }

    pub fn dispatch(&self, message: &mut Message) {
        let code = message.get_message_code();

        if let Some(handler) = self.handlers.get_handler(code) {
            message.set_pointer(8);
            handler.handle(message, self.sender.clone());
        } else {
            warn!(
                "No handler found for message code: {:?}",
                message.get_message_code()
            );
        }
    }
}
