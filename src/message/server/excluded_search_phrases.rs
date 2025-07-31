use crate::{debug, error, info, trace, warn};
use crate::{
    message::{Message, MessageHandler},
    server::ServerOperation,
};
use std::sync::mpsc::Sender;

pub struct ExcludedSearchPhrasesHandler;

impl MessageHandler<ServerOperation> for ExcludedSearchPhrasesHandler {
    fn get_code(&self) -> u8 {
        160
    }

    fn handle(&self, message: &mut Message, _sender: Sender<ServerOperation>) {
        let item_count = message.read_int32();

        let mut exluded_phrases: Vec<String> = Vec::new();
        for _ in 0..item_count {
            // Read the file name, size, and path (structure can vary)
            let phrase = message.read_string();
            exluded_phrases.push(phrase);
        }
        debug!("Excluded search phrases: {:?}", exluded_phrases);
    }
}
