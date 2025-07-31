use std::collections::HashMap;

use crate::message::Message;
use std::sync::mpsc::Sender;

pub trait MessageHandler<Op> {
    fn get_code(&self) -> u8;
    fn handle(&self, message: &mut Message, sender: Sender<Op>);
}
pub struct Handlers<Op> {
    handlers: HashMap<u8, Box<dyn MessageHandler<Op>>>,
}

impl<Op> Default for Handlers<Op> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Op> Handlers<Op> {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register_handler<H>(&mut self, handler: H) -> &mut Self
    where
        H: 'static + MessageHandler<Op> + Send + Sync,
    {
        self.handlers.insert(handler.get_code(), Box::new(handler));
        self
    }
    pub fn get_handler(&self, code: u8) -> Option<&dyn MessageHandler<Op>> {
        self.handlers.get(&code).map(|v| &**v)
    }
}
