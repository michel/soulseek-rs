use crate::message::{handlers::Handlers, Message};
use std::sync::mpsc::Sender;

use crate::warn;

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
        message.set_pointer(4);
        let code = message.read_int32();

        if let Some(handler) = self.handlers.get_handler(code) {
            handler.handle(message, self.sender.clone());
        } else {
            warn!("No handler found for peer message code: {}", code);
        }
    }
}
