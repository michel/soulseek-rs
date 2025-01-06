use crate::message::{handlers::Handlers, message::Message};
use std::sync::mpsc::Sender;

/// Add `<Op>` to make it generic over any operation type.
pub struct MessageDispatcher<Op> {
    sender: Sender<Op>,
    handlers: Handlers<Op>,
}

impl<Op> MessageDispatcher<Op> {
    /// The constructor now takes a `Sender<Op>` and a `Handlers<Op>`
    pub fn new(sender: Sender<Op>, handlers: Handlers<Op>) -> Self {
        MessageDispatcher { sender, handlers }
    }

    /// Our `dispatch` function is the same, but uses our generic types
    pub fn dispatch(&self, message: &mut Message) {
        let code = message.get_message_code();
        if let Some(handler) = self.handlers.get_handler(code) {
            message.set_pointer(8);
            // `handler.handle` takes `Sender<Op>`, so we clone `self.sender`.
            handler.handle(message, self.sender.clone());
        } else {
            println!("No handler found for message code: {:?}", code);
        }
    }
}
