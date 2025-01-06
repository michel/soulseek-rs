use std::collections::HashMap;

use crate::{message::Message, server::ServerOperation};
use std::sync::mpsc::Sender;

use super::server::{
    ConnectToPeerHandler, ExcludedSearchPhrasesHandler, FileSearch, LoginHandler, MessageUser,
    ParentMinSpeedHandler, ParentSpeedRatioHandler, PrivilegedUsersHandler, RoomListHandler,
    WishListIntervalHandler,
};

pub trait MessageHandler<Op> {
    fn get_code(&self) -> u8;
    fn handle(&self, message: &mut Message, sender: Sender<Op>);
}
pub struct Handlers<Op> {
    handlers: HashMap<u8, Box<dyn MessageHandler<Op>>>,
}

impl<Op> Handlers<Op> {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn new_with_server_handlers() -> Self {
        let mut handlers = Self::new();
        handlers.register_handler(LoginHandler);
        handlers.register_handler(RoomListHandler);
        handlers.register_handler(ExcludedSearchPhrasesHandler);
        handlers.register_handler(PrivilegedUsersHandler);
        handlers.register_handler(MessageUser);
        handlers.register_handler(WishListIntervalHandler);
        handlers.register_handler(ParentMinSpeedHandler);
        handlers.register_handler(ParentSpeedRatioHandler);
        handlers.register_handler(PrivilegedUsersHandler);
        handlers.register_handler(FileSearch);
        handlers.register_handler(ConnectToPeerHandler);
        handlers
    }

    /// Register a handler for this particular `Op`.
    pub fn register_handler<H>(&mut self, handler: H) -> &mut Self
    where
        H: 'static + MessageHandler<Op> + Send + Sync,
    {
        self.handlers.insert(handler.get_code(), Box::new(handler));
        self
    }
    pub fn get_handler(&self, code: u8) -> Option<&Box<dyn MessageHandler<Op>>> {
        self.handlers.get(&code)
    }
}
