//! The distributed search network's wire format.
//!
//! Frames carry a one-byte code, unlike the four-byte server and peer codes,
//! and a leaf only ever reads them: 3 Search, 4 BranchLevel, 5 BranchRoot,
//! 93 EmbeddedMessage.

use crate::message::Message;
use crate::message::server::MessageFactory;

/// A search relayed through the tree carries this in its first field; anything
/// else is a frame we do not understand and must not answer.
const SEARCH_IDENTIFIER: u32 = 49;

/// What a parent sends a leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Distributed {
    Search {
        username: String,
        token: u32,
        query: String,
    },
    BranchLevel(i32),
    BranchRoot(String),
}

/// Decode one framed distributed message (length prefix included), or `None`
/// for a ping, an unknown code or a truncated frame.
#[must_use]
pub fn parse(message: &mut Message) -> Option<Distributed> {
    if message.get_size() < 5 {
        return None;
    }
    message.set_pointer(5);
    parse_at(message, message.get_init_code())
}

/// Decode the body that follows a one-byte `code`, the pointer already past
/// it. A code-93 envelope is unwrapped exactly once, as Nicotine+ does: an
/// envelope inside an envelope is nobody's search.
#[must_use]
pub fn parse_at(message: &mut Message, code: u8) -> Option<Distributed> {
    let code = if code == 93 {
        message.read_int8()
    } else {
        code
    };
    match code {
        3 => {
            if remaining(message) < 4
                || message.read_int32() != SEARCH_IDENTIFIER
            {
                return None;
            }
            let username = message.read_string();
            if remaining(message) < 4 {
                return None;
            }
            let token = message.read_int32();
            let query = message.read_string();
            (!username.is_empty() && !query.is_empty()).then_some(
                Distributed::Search {
                    username,
                    token,
                    query,
                },
            )
        }
        4 => (remaining(message) >= 4)
            .then(|| Distributed::BranchLevel(message.read_int32() as i32)),
        5 => {
            let root = message.read_string();
            (!root.is_empty()).then_some(Distributed::BranchRoot(root))
        }
        _ => None,
    }
}

const fn remaining(message: &mut Message) -> usize {
    message.get_size().saturating_sub(message.get_pointer())
}

/// A distributed frame with the one-byte `code`: what a parent (or a test
/// standing in for one) sends down the tree.
fn framed(code: u8, body: impl FnOnce(&mut Message)) -> Message {
    let mut message = Message::new();
    message.write_int8(code);
    body(&mut message);
    message
}

#[must_use]
pub fn build_search(username: &str, token: u32, query: &str) -> Message {
    framed(3, |m| {
        m.write_int32(SEARCH_IDENTIFIER)
            .write_string(username)
            .write_int32(token)
            .write_string(query);
    })
}

#[must_use]
pub fn build_branch_level(level: i32) -> Message {
    framed(4, |m| {
        m.write_int32(level as u32);
    })
}

/// What we tell the server about our place in the tree: the four messages
/// Nicotine+ sends together whenever it changes.
#[must_use]
pub fn stance(root: &str, level: u32, has_parent: bool) -> Vec<Message> {
    vec![
        MessageFactory::build_have_no_parent(!has_parent),
        MessageFactory::build_branch_root(root),
        MessageFactory::build_branch_level(level),
        MessageFactory::build_accept_children(false),
    ]
}

#[must_use]
pub fn build_branch_root(root: &str) -> Message {
    framed(5, |m| {
        m.write_string(root);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_distributed_search_carries_who_asked_and_what_for() {
        let frame = build_search("seeker", 77, "aphex twin");
        let mut message = Message::new_with_data(frame.get_buffer());
        assert_eq!(
            parse(&mut message),
            Some(Distributed::Search {
                username: "seeker".to_string(),
                token: 77,
                query: "aphex twin".to_string(),
            })
        );
    }

    #[test]
    fn a_search_with_the_wrong_identifier_is_dropped() {
        let mut body = Message::new();
        body.write_int8(3)
            .write_int32(7)
            .write_string("seeker")
            .write_int32(1)
            .write_string("q");
        let mut message = Message::new_with_data(body.get_buffer());
        assert_eq!(parse(&mut message), None);
    }

    #[test]
    fn branch_level_and_root_frames_parse() {
        let mut level =
            Message::new_with_data(build_branch_level(2).get_buffer());
        assert_eq!(parse(&mut level), Some(Distributed::BranchLevel(2)));
        let mut root =
            Message::new_with_data(build_branch_root("rooty").get_buffer());
        assert_eq!(
            parse(&mut root),
            Some(Distributed::BranchRoot("rooty".to_string()))
        );
    }

    #[test]
    fn an_embedded_search_unwraps_to_the_search_itself() {
        let inner = build_search("seeker", 5, "q").get_data();
        let mut body = Message::new();
        body.write_int8(93).write_raw_bytes(inner);
        let mut message = Message::new_with_data(body.get_buffer());
        assert!(matches!(
            parse(&mut message),
            Some(Distributed::Search { token: 5, .. })
        ));
    }

    #[test]
    fn unknown_or_truncated_frames_parse_to_nothing() {
        let mut ping =
            Message::new_with_data(Message::new().write_int8(0).get_buffer());
        assert_eq!(parse(&mut ping), None);
        let mut short = Message::new_with_data(vec![1, 0, 0, 0, 3]);
        assert_eq!(parse(&mut short), None);
        let mut level = Message::new_with_data(vec![3, 0, 0, 0, 4, 1, 0]);
        assert_eq!(parse(&mut level), None, "a level needs four bytes");
        let mut root =
            Message::new_with_data(build_branch_root("").get_buffer());
        assert_eq!(parse(&mut root), None, "an empty root is no root");
        let mut blank =
            Message::new_with_data(build_search("seeker", 1, "").get_buffer());
        assert_eq!(parse(&mut blank), None, "an empty query is no search");
    }

    // An envelope inside an envelope is unwrapped once and found empty; a
    // frame that is nothing but envelopes must not recurse its way off the
    // stack.
    #[test]
    fn a_nested_envelope_is_not_unwrapped_twice() {
        let mut nested = Message::new();
        nested.write_int8(93).write_raw_bytes(
            Message::new()
                .write_int8(93)
                .write_raw_bytes(build_search("seeker", 5, "q").get_data())
                .get_data(),
        );
        let mut message = Message::new_with_data(nested.get_buffer());
        assert_eq!(parse(&mut message), None);

        let mut onions = Message::new_with_data(
            Message::new()
                .write_raw_bytes(vec![93u8; 64 * 1024])
                .get_buffer(),
        );
        assert_eq!(parse(&mut onions), None);
    }

    // The frame as the protocol documents it, byte for byte: code 3, the
    // constant 49, then user, token and query.
    #[test]
    fn a_search_frame_matches_the_documented_layout() {
        let mut bytes = vec![3u8, 49, 0, 0, 0];
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(b"me");
        bytes.extend(9u32.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(b"q");
        let mut framed = (bytes.len() as u32).to_le_bytes().to_vec();
        framed.extend(&bytes);
        assert_eq!(build_search("me", 9, "q").get_buffer(), framed);
        assert_eq!(
            parse(&mut Message::new_with_data(framed)),
            Some(Distributed::Search {
                username: "me".to_string(),
                token: 9,
                query: "q".to_string()
            })
        );
    }
}
