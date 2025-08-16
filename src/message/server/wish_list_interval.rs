use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    message::{Message, MessageHandler},
    server::ServerOperation,
};

pub struct WishListIntervalHandler;

// The server tells us the wishlist search interval.
// This interval is almost always 12 minutes, or 2 minutes for privileged users.
impl MessageHandler<ServerOperation> for WishListIntervalHandler {
    fn get_code(&self) -> u32 {
        104
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let number = message.read_int32();
        debug!("Wishlist search interval: {} in seconds", number);
    }
}
