use crate::message::{Message, MessageHandler};
use crate::peer::Peer;
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;
pub struct ConnectToPeerHandler;
impl MessageHandler<ServerOperation> for ConnectToPeerHandler {
    fn get_code(&self) -> u8 {
        18
    }
    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let peer = Peer::new_from_message(message);
        match peer.connection_type {
            crate::peer::ConnectionType::P => (),
            crate::peer::ConnectionType::F => debug!("{:?}", peer),
            crate::peer::ConnectionType::D => (),
        }
        sender.send(ServerOperation::ConnectToPeer(peer)).unwrap();
    }
}
