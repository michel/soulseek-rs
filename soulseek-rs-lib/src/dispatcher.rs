use crate::message::{Message, handlers::Handlers};

use crate::warn;

pub struct MessageDispatcher<Op> {
    owner_name: String,
    handlers: Handlers<Op>,
}

impl<Op> MessageDispatcher<Op> {
    #[must_use]
    pub const fn new(owner_name: String, handlers: Handlers<Op>) -> Self {
        Self {
            owner_name,
            handlers,
        }
    }

    pub fn dispatch(&self, message: &mut Message) -> Vec<Op> {
        let code = message.get_message_code();

        if let Some(handler) = self.handlers.get_handler(code) {
            message.set_pointer(8);
            let mut out = Vec::new();
            handler.handle(message, &mut out);
            out
        } else {
            warn!(
                "[{}:dispatcher] No handler found for message code: {}",
                self.owner_name,
                message.get_message_code()
            );
            Vec::new()
        }
    }
}
