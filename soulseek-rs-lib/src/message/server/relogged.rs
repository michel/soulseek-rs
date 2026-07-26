use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

/// `Relogged` (code 41): the server is about to cut this connection because
/// the same username logged in somewhere else.
pub struct ReloggedHandler;

impl MessageHandler<ServerMessage> for ReloggedHandler {
    fn get_code(&self) -> u8 {
        41
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let _ = message;
        let _ = sender.send(ServerMessage::Relogged);
    }
}
