use crate::message::{Message, MessageHandler};
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;

pub struct GetPeerAddressHandler;

impl MessageHandler<ServerOperation> for GetPeerAddressHandler {
    fn get_code(&self) -> u8 {
        3
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let username = message.read_string();

        // Read IP address as 4 bytes
        let mut ip: Vec<u8> = vec![];
        for _ in 0..4 {
            ip.push(message.read_int8());
        }
        let host = format!("{}.{}.{}.{}", ip[3], ip[2], ip[1], ip[0]);

        let port = message.read_int32();
        let obfuscation_type = message.read_int32();
        let obfuscated_port = message.read_int32() as u16;

        sender
            .send(ServerOperation::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            })
            .unwrap();
    }
}
