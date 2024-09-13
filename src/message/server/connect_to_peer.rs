use crate::message::{Message, MessageHandler};
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;
pub struct ConnectToPeerHandler;
impl MessageHandler for ConnectToPeerHandler {
    fn get_code(&self) -> u8 {
        18
    }
    fn handle(&self, _message: &mut Message, _sender: Sender<ServerOperation>) {
        println!("Handling ConnectToPeer message");
        // let username = message.read_string();
        // let connection_type = message.read_string();
        // let mut ip: Vec<i8> = vec![];
        // for _ in 0..4 {
        //     ip.push(message.read_int8());
        // }
        // let host: String = format!("{}.{}.{}.{}", ip[3], ip[2], ip[1], ip[0]);
        //
        // let (port, token, privileged, unknown, obfuscated_port) = (
        //     message.read_int32(),
        //     message.read_int32(),
        //     message.read_int8(),
        //     message.read_int8(),
        //     message.read_int8(),
        // );
        //
        // let peer = Peer::new(
        //     username,
        //     connection_type,
        //     host,
        //     port,
        //     token,
        //     privileged,
        //     unknown,
        //     obfuscated_port,
        // );
        //
        // sender.send(ServerOperation::ConnectToPeer(peer)).unwrap();
    }
}
