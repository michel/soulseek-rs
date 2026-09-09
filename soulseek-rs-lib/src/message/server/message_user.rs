use crate::actor::server_actor::{ServerMessage, UserMessage};
use crate::info;
use crate::message::server::MessageFactory;
use crate::message::{Message, MessageHandler};

use std::sync::mpsc::Sender;

pub struct MessageUser;

impl MessageHandler<ServerMessage> for MessageUser {
    fn get_code(&self) -> u32 {
        22
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let id = message.read_int32();
        let timestamp = message.read_int32();
        let username = message.read_string();
        let message_content = message.read_string();
        let new_message = message.read_bool();
        let user_message = UserMessage::new(
            id,
            timestamp,
            username,
            message_content,
            new_message,
        );

        info!("[MessageUser] User message received:{:?}", user_message);

        // Acknowledge every message, not only the ones flagged new. The flag
        // marks a message delivered as it was sent; one the server stored
        // while we were offline arrives with it false, and those are exactly
        // the messages the server keeps re-sending at every login until they
        // are acknowledged.
        let _ = sender.send(ServerMessage::SendMessage(
            MessageFactory::build_message_acked(id),
        ));

        // Surface the message to the client so it can be read via the API.
        let _ =
            sender.send(ServerMessage::PrivateMessageReceived(user_message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    fn deliver(new_message: bool) -> Vec<ServerMessage> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(7);
            m.write_int32(1_700_000_000);
            m.write_string("alice");
            m.write_string("hello");
            m.write_int8(u8::from(new_message));
        });
        MessageUser.handle(&mut message, tx);
        rx.try_iter().collect()
    }

    fn acked_id(messages: &[ServerMessage]) -> Option<u32> {
        messages.iter().find_map(|message| match message {
            ServerMessage::SendMessage(m) => {
                let mut decoded = Message::new_with_data(m.get_buffer());
                (decoded.get_message_code() == 23).then(|| {
                    decoded.set_pointer(8);
                    decoded.read_int32()
                })
            }
            _ => None,
        })
    }

    #[test]
    fn a_freshly_delivered_message_is_acknowledged() {
        let sent = deliver(true);
        assert_eq!(acked_id(&sent), Some(7));
    }

    #[test]
    fn a_message_stored_while_we_were_offline_is_acknowledged_too() {
        // Stored messages arrive with new_message false; without an ack the
        // server re-delivers them at every login, forever.
        let sent = deliver(false);
        assert_eq!(acked_id(&sent), Some(7));
    }

    #[test]
    fn the_message_itself_is_surfaced_to_the_client() {
        let sent = deliver(false);
        assert!(sent.iter().any(|message| matches!(
            message,
            ServerMessage::PrivateMessageReceived(m)
                if m.message() == "hello" && m.username() == "alice"
        )));
    }
}
