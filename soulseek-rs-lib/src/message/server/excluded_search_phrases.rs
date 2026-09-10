use crate::debug;
use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};
use std::sync::mpsc::Sender;

pub struct ExcludedSearchPhrasesHandler;

impl MessageHandler<ServerMessage> for ExcludedSearchPhrasesHandler {
    fn get_code(&self) -> u32 {
        160
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
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
        // Kept, not just logged: the server rejects a search carrying one of
        // these, so a client that forgets them spends its search allowance on
        // queries that were never going to be answered.
        let _ =
            sender.send(ServerMessage::ExcludedSearchPhrases(exluded_phrases));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn hostile_item_count_does_not_hang() {
        // item_count=u32::MAX with no phrases: the guard must make this return
        // promptly instead of looping ~4 billion times.
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(u32::MAX);
        });
        ExcludedSearchPhrasesHandler.handle(&mut message, tx);
    }

    #[test]
    fn the_phrases_are_reported_not_only_logged() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(2);
            m.write_string("banned");
            m.write_string("blocked");
        });

        ExcludedSearchPhrasesHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::ExcludedSearchPhrases(phrases)) => {
                assert_eq!(phrases, ["banned", "blocked"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
