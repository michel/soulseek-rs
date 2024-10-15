use super::handlers::MessageHandler;
use crate::{client::ClientOperation, message::Message, peer::peer::Peer, server::Context};
use std::sync::{Arc, Mutex};

pub struct ConnectToPeerHandler;
impl MessageHandler for ConnectToPeerHandler {
    fn get_code(&self) -> u8 {
        18
    }
    fn handle(&self, message: &mut Message, context: Arc<Mutex<Context>>) {
        println!("Handling ConnectToPeer message");
        let username = message.read_string();
        let connection_type = message.read_string();
        let mut ip: Vec<i8> = vec![];
        for _ in 0..4 {
            ip.push(message.read_int8());
        }
        let host: String = format!("{}.{}.{}.{}", ip[3], ip[2], ip[1], ip[0]);

        let (port, token, privileged, unknown, obfuscated_port) = (
            message.read_int32(),
            message.read_int32(),
            message.read_int8(),
            message.read_int8(),
            message.read_int8(),
        );

        let peer = Peer::new(
            username,
            connection_type,
            host,
            port,
            token,
            privileged,
            unknown,
            obfuscated_port,
        );

        context
            .lock()
            .unwrap()
            .client_channel
            .send(ClientOperation::ConnectToPeer(peer))
            .unwrap_or_default();
    }
}
