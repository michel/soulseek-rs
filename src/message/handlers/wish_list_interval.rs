use std::sync::{mpsc::Sender, Arc, Condvar, Mutex};

use crate::{message::Message, server::ServerOperation};

use super::handlers::MessageHandler;
pub struct WishListIntervalHandler;

// The server tells us the wishlist search interval.
// This interval is almost always 12 minutes, or 2 minutes for privileged users.
impl MessageHandler for WishListIntervalHandler {
    fn get_code(&self) -> u8 {
        104
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let number = message.read_int32();
        println!("Wishlist search interval: {} in seconds", number);
    }
}
