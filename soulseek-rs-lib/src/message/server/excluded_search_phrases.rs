use crate::debug;
use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

pub struct ExcludedSearchPhrasesHandler;

impl MessageHandler<ServerMessage> for ExcludedSearchPhrasesHandler {
    fn get_code(&self) -> u8 {
        160
    }

    fn handle(&self, message: &mut Message, _out: &mut Vec<ServerMessage>) {
        let item_count = message.read_int32();

        let mut exluded_phrases: Vec<String> = Vec::new();
        for _ in 0..item_count {
            // Guard against a hostile item_count outrunning the payload.
            if message.get_pointer() + 4 > message.get_size() {
                break;
            }
            let phrase = message.read_string();
            exluded_phrases.push(phrase);
        }
        debug!("Excluded search phrases: {:?}", exluded_phrases);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_item_count_does_not_hang() {
        // item_count=u32::MAX with no phrases: the guard must make this return
        // promptly instead of looping ~4 billion times.
        let mut out = Vec::new();
        let mut message = Message::new();
        message.write_raw_bytes(vec![0u8; 8]);
        message.write_int32(u32::MAX);
        message.set_pointer(8);
        ExcludedSearchPhrasesHandler.handle(&mut message, &mut out);
    }
}
