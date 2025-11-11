use crate::debug;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct WishListIntervalHandler;

// The server tells us the wishlist search interval.
// This interval is almost always 12 minutes, or 2 minutes for privileged users.
impl MessageHandler<ServerMessage> for WishListIntervalHandler {
    fn get_code(&self) -> u8 {
        104
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerMessage>) {
        let number = message.read_int32();
        debug!("Wishlist search interval: {} in seconds", number);
    }
}
