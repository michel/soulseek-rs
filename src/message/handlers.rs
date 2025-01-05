use std::collections::HashMap;

use crate::{message::Message, server::ServerOperation};
use std::sync::mpsc::Sender;

use super::server::{
    ConnectToPeerHandler, ExcludedSearchPhrasesHandler, FileSearch, LoginHandler, MessageUser,
    ParentMinSpeedHandler, ParentSpeedRatioHandler, PrivilegedUsersHandler, RoomListHandler,
    WishListIntervalHandler,
};

pub trait MessageHandler {
    fn get_code(&self) -> u8;
    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>);
}
pub struct Handlers {
    handlers: HashMap<u8, Box<dyn MessageHandler>>,
}

impl Handlers {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
    pub fn new_with_server_handlers() -> Self {
        let mut handlers = Self::new();
        handlers.add_server_handlers();
        handlers
    }

    fn add_server_handlers(&mut self) {
        self.register_handler(LoginHandler);
        self.register_handler(RoomListHandler);
        self.register_handler(ExcludedSearchPhrasesHandler);
        self.register_handler(PrivilegedUsersHandler);
        self.register_handler(MessageUser);
        self.register_handler(WishListIntervalHandler);
        self.register_handler(ParentMinSpeedHandler);
        self.register_handler(ParentSpeedRatioHandler);
        self.register_handler(PrivilegedUsersHandler);
        self.register_handler(FileSearch);
        self.register_handler(ConnectToPeerHandler);
    }

    pub fn register_handler<H: 'static + MessageHandler + Send + Sync>(
        &mut self,
        handler: H,
    ) -> &mut Self {
        self.handlers.insert(handler.get_code(), Box::new(handler));
        self
    }
    pub fn get_handler(&self, code: u8) -> Option<&Box<dyn MessageHandler>> {
        self.handlers.get(&code)
    }
}
