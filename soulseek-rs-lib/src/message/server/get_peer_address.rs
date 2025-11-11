use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

pub struct GetPeerAddressHandler;

impl MessageHandler<ServerMessage> for GetPeerAddressHandler {
    fn get_code(&self) -> u8 {
        3
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
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
        println!("GetPeerAddressHandler: {:?}", username); // Debug print

        sender
            .send(ServerMessage::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            })
            .unwrap();
    }
}
