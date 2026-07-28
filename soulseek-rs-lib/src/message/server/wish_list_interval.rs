use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

/// `WishlistInterval` (code 104): how often the server will accept a wishlist
/// search from us. Usually 720 seconds, or 120 for a privileged account.
///
/// The value is the server's rate limit, not a suggestion, so it is kept rather
/// than logged: the re-search timer runs off it.
pub struct WishListIntervalHandler;

impl MessageHandler<ServerMessage> for WishListIntervalHandler {
    fn get_code(&self) -> u8 {
        104
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let seconds = message.read_int32();
        let _ = sender.send(ServerMessage::WishlistInterval(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_announced_interval_is_reported_not_discarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(720);
        });

        WishListIntervalHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::WishlistInterval(seconds)) => {
                assert_eq!(seconds, 720);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
