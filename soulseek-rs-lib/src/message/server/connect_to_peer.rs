use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};
use crate::peer::Peer;
use std::sync::mpsc::Sender;
pub struct ConnectToPeerHandler;

impl MessageHandler<ServerMessage> for ConnectToPeerHandler {
    fn get_code(&self) -> u8 {
        18
    }
    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let Some(peer) = Peer::new_from_message(message) else {
            return;
        };
        let _ = sender.send(ServerMessage::ConnectToPeer(peer));
    }
}
