//! What the server says about the distributed network: parent candidates
//! (code 102), a reset (130), and a search embedded for a branch root (93).

use crate::actor::server_actor::ServerMessage;
use crate::message::distributed::{Distributed, parse_at};
use crate::message::{Message, MessageHandler};
use std::net::Ipv4Addr;
use std::sync::mpsc::Sender;

pub struct PossibleParentsHandler;
impl MessageHandler<ServerMessage> for PossibleParentsHandler {
    fn get_code(&self) -> u32 {
        102
    }
    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let count = message.read_int32();
        let candidates = (0..count.min(10))
            .map_while(|_| entry(message))
            .filter_map(|(username, host, port)| {
                Some((username, host, u16::try_from(port).ok()?))
            })
            .collect();
        let _ = sender.send(ServerMessage::PossibleParents(candidates));
    }
}

/// One whole `(username, host, port)` entry, or `None` where the message
/// runs out: a truncated entry is not a parent to dial.
fn entry(message: &mut Message) -> Option<(String, String, u32)> {
    let username = message.read_string();
    if username.is_empty()
        || message.get_size().saturating_sub(message.get_pointer()) < 8
    {
        return None;
    }
    let host = Ipv4Addr::from(message.read_int32()).to_string();
    Some((username, host, message.read_int32()))
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

    // The count says two, the bytes hold one and a half: the whole one is
    // a candidate, the stub is not. A port that is not a port is skipped too.
    #[test]
    fn a_truncated_or_absurd_entry_is_not_a_candidate() {
        let (sender, received) = channel();
        let mut message = crate::message::framed(|m| {
            m.write_int32(3)
                .write_string("alice")
                .write_int32(u32::from_le_bytes([1, 0, 0, 127]))
                .write_int32(70_000)
                .write_string("bob")
                .write_int32(u32::from_le_bytes([8, 8, 8, 8]))
                .write_int32(2235)
                .write_string("carol");
        });
        PossibleParentsHandler.handle(&mut message, sender);
        let ServerMessage::PossibleParents(candidates) =
            received.try_recv().unwrap()
        else {
            panic!("expected PossibleParents");
        };
        assert_eq!(
            candidates,
            vec![("bob".to_string(), "8.8.8.8".to_string(), 2235)]
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
