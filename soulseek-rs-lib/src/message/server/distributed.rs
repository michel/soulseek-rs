//! What the server says about the distributed network: parent candidates
//! (code 102), a reset (130), and a search embedded for a branch root (93).

use crate::actor::server_actor::ServerMessage;
use crate::message::distributed::{Distributed, parse_at};
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

pub struct PossibleParentsHandler;
impl MessageHandler<ServerMessage> for PossibleParentsHandler {
    fn get_code(&self) -> u32 {
        102
    }
    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let count = message.read_int32();
        let mut candidates = Vec::new();
        for _ in 0..count.min(10) {
            if message.get_pointer() >= message.get_size() {
                break;
            }
            let username = message.read_string();
            // The address travels as four bytes in network order.
            let ip: Vec<u8> = (0..4).map(|_| message.read_int8()).collect();
            let host = format!("{}.{}.{}.{}", ip[3], ip[2], ip[1], ip[0]);
            let port = message.read_int32() as u16;
            candidates.push((username, host, port));
        }
        let _ = sender.send(ServerMessage::PossibleParents(candidates));
    }
}

pub struct ResetDistributedHandler;
impl MessageHandler<ServerMessage> for ResetDistributedHandler {
    fn get_code(&self) -> u32 {
        130
    }
    fn handle(&self, _message: &mut Message, sender: Sender<ServerMessage>) {
        let _ = sender.send(ServerMessage::ResetDistributed);
    }
}

/// A search the server hands us directly because we are a branch root; it is
/// answered like any other.
pub struct EmbeddedMessageHandler;
impl MessageHandler<ServerMessage> for EmbeddedMessageHandler {
    fn get_code(&self) -> u32 {
        93
    }
    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let code = message.read_int8();
        if let Some(Distributed::Search {
            username,
            token,
            query,
        }) = parse_at(message, code)
        {
            let _ = sender.send(ServerMessage::FileSearchRequest {
                username,
                token,
                query,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn possible_parents_carry_each_candidates_address() {
        let (sender, received) = channel();
        let mut message = crate::message::framed(|m| {
            m.write_int32(2)
                .write_string("alice")
                .write_int32(u32::from_le_bytes([1, 0, 0, 127]))
                .write_int32(2234)
                .write_string("bob")
                .write_int32(u32::from_le_bytes([8, 8, 8, 8]))
                .write_int32(2235);
        });
        PossibleParentsHandler.handle(&mut message, sender);
        let ServerMessage::PossibleParents(candidates) =
            received.try_recv().unwrap()
        else {
            panic!("expected PossibleParents");
        };
        assert_eq!(
            candidates,
            vec![
                ("alice".to_string(), "127.0.0.1".to_string(), 2234),
                ("bob".to_string(), "8.8.8.8".to_string(), 2235),
            ]
        );
    }

    #[test]
    fn a_reset_and_an_embedded_search_become_client_events() {
        let (sender, received) = channel();
        let mut reset = crate::message::framed(|_| {});
        ResetDistributedHandler.handle(&mut reset, sender.clone());
        assert!(matches!(
            received.try_recv().unwrap(),
            ServerMessage::ResetDistributed
        ));

        let inner = crate::message::distributed::build_search("seeker", 9, "q")
            .get_data();
        let mut embedded = crate::message::framed(|m| {
            m.write_raw_bytes(inner);
        });
        EmbeddedMessageHandler.handle(&mut embedded, sender);
        let ServerMessage::FileSearchRequest {
            username,
            token,
            query,
        } = received.try_recv().unwrap()
        else {
            panic!("expected FileSearchRequest");
        };
        assert_eq!(
            (username.as_str(), token, query.as_str()),
            ("seeker", 9, "q")
        );
    }
}
