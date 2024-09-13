use super::handlers::MessageHandler;
use crate::{message::Message, server::Context};
use std::sync::{Arc, Mutex};

pub struct ExcludedSearchPhrasesHandler;

impl MessageHandler for ExcludedSearchPhrasesHandler {
    fn get_code(&self) -> u8 {
        160
    }

    fn handle(&self, message: &mut Message, _context: Arc<Mutex<Context>>) {
        let item_count = message.read_int32();

        let mut exluded_phrases: Vec<String> = Vec::new();
        for _ in 0..item_count {
            // Read the file name, size, and path (structure can vary)
            let phrase = message.read_string();
            exluded_phrases.push(phrase);
        }
        println!("Excluded search phrases: {:?}", exluded_phrases);
    }
}
