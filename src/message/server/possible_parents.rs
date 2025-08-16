use crate::debug;
use crate::message::server::MessageFactory;
use crate::message::{Message, MessageHandler};
use crate::server::ServerOperation;
use std::sync::mpsc::Sender;

pub struct PossibleParentsHandler;

impl MessageHandler<ServerOperation> for PossibleParentsHandler {
    fn get_code(&self) -> u32 {
        102
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let number_of_parents = message.read_int32();
        debug!(
            "Received {} possible parents from server.",
            number_of_parents
        );

        for _ in 0..number_of_parents {
            let username = message.read_string();
            let mut ip_parts: Vec<u8> = Vec::new();
            for _ in 0..4 {
                ip_parts.push(message.read_int8());
            }
            let host = format!(
                "{}.{}.{}.{}",
                ip_parts[3], ip_parts[2], ip_parts[1], ip_parts[0]
            );
            let port = message.read_int32();

            // 1. Acknowledge the parent by sending their IP back to the server
            let parent_ip_message =
                MessageFactory::build_parent_ip_message(ip_parts.clone());
            sender
                .send(ServerOperation::SendMessage(parent_ip_message))
                .unwrap();

            // 2. The Node.js client also connects to this peer as a distributed node.
            //    For now, we can skip this step to keep the MVP simple. The acknowledgement
            //    is likely the most critical part.
            debug!("Acknowledged parent {} at {}:{}", username, host, port);
        }
    }
}
