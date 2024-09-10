use std::sync::{Arc, Mutex};

use crate::{dispatcher::MessageHandler, message::Message, server::Context};
pub struct ExcludedSearchPhrasesHandler;

impl MessageHandler for ExcludedSearchPhrasesHandler {
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
