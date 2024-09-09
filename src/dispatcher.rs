use crate::message::handlers::excluded_search_phrases::ExcludedSearchPhrasesHandler;
use crate::message::handlers::login::LoginHandler;
use crate::message::handlers::privileged_users::PrivilegedUsersHandler;
use crate::message::handlers::room_list::RoomListHandler;
use crate::{
    message::{Message, MessageType},
    server::Context,
};
use std::collections::HashMap;

pub trait MessageHandler {
    fn handle(&self, message: &mut Message, context: Context);
}

pub struct MessageDispatcher {
    handlers: HashMap<MessageType, Box<dyn MessageHandler + Send + Sync>>,
    context: Context,
}

impl MessageDispatcher {
    pub fn new(context: Context) -> Self {
        let mut dispatcher = Self {
            handlers: HashMap::new(),
            context,
        };
        dispatcher.register_default_handlers();
        dispatcher
    }

    pub fn register_handler<H: 'static + MessageHandler + Send + Sync>(
        &mut self,
        msg_type: MessageType,
        handler: H,
    ) -> &mut Self {
        self.handlers.insert(msg_type, Box::new(handler));
        self
    }

    pub fn dispatch(&self, message: &mut Message) {
        if let Some(handler) = self.handlers.get(&message.get_message_type()) {
            message.set_pointer(8);
            return handler.handle(message, self.context.clone());
        }
        println!(
            "No handler found for message type: {:?}",
            message.get_message_type()
        );
    }
    pub fn register_default_handlers(&mut self) {
        self.register_handler(MessageType::Login, LoginHandler);
        self.register_handler(MessageType::PrivilegedUsers, PrivilegedUsersHandler);
        self.register_handler(
            MessageType::ExcludedSearchPhrases,
            ExcludedSearchPhrasesHandler,
        );
        self.register_handler(MessageType::RoomList, RoomListHandler);
    }
}
