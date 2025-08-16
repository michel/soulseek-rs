use crate::client::ClientOperation;
use crate::message::{Message, MessageHandler};
use crate::{debug, info};
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct SearchRequestInfo {
    pub username: String,
    pub ticket: Vec<u8>,
    pub query: String,
}

pub struct SearchRequest;

impl MessageHandler<ClientOperation> for SearchRequest {
    fn get_code(&self) -> u32 {
        3
    }

    fn handle(&self, message: &mut Message, sender: Sender<ClientOperation>) {
        message.set_pointer(4);
        let code = message.read_int8();
        if code != 3 {
            debug!("Expected SearchRequest code 3, got {}", code);
            return;
        }

        let _unknown = message.read_int32();
        let username = message.read_string();
        let ticket = message.read_raw_bytes(4);
        let query = message.read_string();

        info!("Received distributed search from {}: '{}'", username, query);

        let search_info = SearchRequestInfo {
            username,
            ticket,
            query,
        };

        let _ = sender.send(ClientOperation::DistributedSearch(search_info));
    }
}
