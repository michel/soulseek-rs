use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct RoomListHandler;

impl MessageHandler<ServerMessage> for RoomListHandler {
    fn get_code(&self) -> u8 {
        64
    }

    fn handle(&self, _message: &mut Message, _sender: Sender<ServerMessage>) {
        // Room listing is not yet exposed to the client; this handler exists
        // so the dispatcher can ack code 64 without logging an unknown message.
    }
}
