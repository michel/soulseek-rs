use crate::message::{Message, handlers::Handlers};
use std::sync::mpsc::Sender;

use crate::debug;

pub struct MessageDispatcher<Op> {
    owner_name: String,
    sender: Sender<Op>,
    handlers: Handlers<Op>,
}

impl<Op> MessageDispatcher<Op> {
    #[must_use]
    pub const fn new(
        owner_name: String,
        sender: Sender<Op>,
        handlers: Handlers<Op>,
    ) -> Self {
        Self {
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
            debug!(
                "[{}:dispatcher] No handler found for message code: {code}",
                self.owner_name
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageHandler;
    use std::sync::mpsc::channel;

    struct Echo(u32);

    impl MessageHandler<u32> for Echo {
        fn get_code(&self) -> u32 {
            self.0
        }
        fn handle(&self, _: &mut Message, sender: Sender<u32>) {
            let _ = sender.send(self.0);
        }
    }

    fn framed(code: u32) -> Message {
        Message::new_with_data(Message::new().write_int32(code).get_buffer())
    }

    #[test]
    fn a_code_above_255_is_not_confused_with_its_low_byte() {
        let (sender, received) = channel();
        let mut handlers = Handlers::new();
        handlers.register_handler(Echo(1));
        handlers.register_handler(Echo(1001));
        let dispatcher =
            MessageDispatcher::new("test".to_string(), sender, handlers);

        dispatcher.dispatch(&mut framed(257));
        dispatcher.dispatch(&mut framed(1001));

        assert_eq!(received.try_iter().collect::<Vec<_>>(), [1001]);
    }
}
