//! Serving children: the other half of the distributed search network.
//!
//! A leaf only reads the tree; a parent also carries it. When the client is
//! configured to accept children, peers dial us with a `D` connection, we tell
//! each one where our branch sits, and every search that reaches us is passed
//! down to all of them. That is the whole duty — a child answers searches
//! itself, and reports its own place to the server.
//!
//! Accepting children is opt-in ([`crate::ClientSettings::accept_children`]):
//! it costs a socket and the network's whole search stream per child, which is
//! not something a client should take on without being asked to.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;

use crate::message::Message;
use crate::message::distributed;
use crate::{debug, trace};

/// How many children to carry at once. Nicotine+ offers ten; the cost is one
/// socket and one copy of every search each, so the cap is what keeps a
/// parent's outbound traffic bounded.
pub const MAX_CHILDREN: usize = 10;

/// The children hanging from us, keyed by username.
pub struct Children {
    links: HashMap<String, TcpStream>,
    max: usize,
    accepting: bool,
}

impl Default for Children {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Children {
    #[must_use]
    pub fn new(accepting: bool) -> Self {
        Self {
            links: HashMap::new(),
            max: MAX_CHILDREN,
            accepting,
        }
    }

    /// Whether we take children at all, and still have room for one.
    #[must_use]
    pub fn has_room(&self) -> bool {
        self.accepting && self.links.len() < self.max
    }

    /// Whether the client accepts children at all, regardless of how full it
    /// is — what the server is told in `AcceptChildren` (code 100).
    #[must_use]
    pub const fn accepting(&self) -> bool {
        self.accepting
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.links.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    #[must_use]
    pub fn usernames(&self) -> Vec<String> {
        let mut names: Vec<String> = self.links.keys().cloned().collect();
        names.sort();
        names
    }

    /// Take on `username` as a child. Refused when we are not accepting or
    /// are full; a second connection from a child we already carry replaces
    /// the first, since the peer only reconnects when it thinks the old one
    /// is gone.
    pub fn accept(&mut self, username: &str, stream: TcpStream) -> bool {
        if !self.accepting {
            return false;
        }
        if self.links.len() >= self.max && !self.links.contains_key(username) {
            debug!(
                "[distributed] refusing child {}: already carrying {}",
                username,
                self.links.len()
            );
            return false;
        }
        self.links.insert(username.to_string(), stream);
        true
    }

    pub fn remove(&mut self, username: &str) {
        if self.links.remove(username).is_some() {
            debug!("[distributed] child {} is gone", username);
        }
    }

    /// Send one frame to `username`, dropping the child if the write fails.
    pub fn send_to(&mut self, username: &str, message: &Message) {
        let failed = match self.links.get_mut(username) {
            Some(stream) => stream.write_all(&message.get_buffer()).is_err(),
            None => return,
        };
        if failed {
            self.remove(username);
        }
    }

    /// Send one frame to every child, dropping those whose write fails: a
    /// child that has gone away must not keep its slot.
    pub fn broadcast(&mut self, message: &Message) {
        let bytes = message.get_buffer();
        let mut dead = Vec::new();
        for (username, stream) in &mut self.links {
            if stream.write_all(&bytes).is_err() {
                dead.push(username.clone());
            }
        }
        for username in dead {
            self.remove(&username);
        }
    }

    /// Tell `username` where our branch sits — what a child needs before it
    /// can report its own place to the server.
    pub fn send_stance_to(&mut self, username: &str, root: &str, level: u32) {
        trace!(
            "[distributed] telling {} we sit at {}/{}",
            username, root, level
        );
        self.send_to(username, &distributed::build_branch_root(root));
        self.send_to(
            username,
            &distributed::build_branch_level(
                i32::try_from(level).unwrap_or(i32::MAX),
            ),
        );
    }

    /// Tell every child our branch moved.
    pub fn broadcast_stance(&mut self, root: &str, level: u32) {
        self.broadcast(&distributed::build_branch_root(root));
        self.broadcast(&distributed::build_branch_level(
            i32::try_from(level).unwrap_or(i32::MAX),
        ));
    }

    /// Pass a search down the tree.
    pub fn broadcast_search(
        &mut self,
        username: &str,
        token: u32,
        query: &str,
    ) {
        if self.links.is_empty() {
            return;
        }
        self.broadcast(&distributed::build_search(username, token, query));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};

    /// A connected socket pair: the child's end and ours.
    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let ours = TcpStream::connect(addr).expect("connect");
        let (theirs, _) = listener.accept().expect("accept");
        (theirs, ours)
    }

    fn read_frames(stream: &mut TcpStream, bytes: usize) -> Vec<u8> {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("timeout");
        let mut buf = vec![0u8; bytes];
        stream.read_exact(&mut buf).expect("read");
        buf
    }

    #[test]
    fn a_client_that_does_not_accept_children_takes_none() {
        let (_child, ours) = pair();
        let mut children = Children::new(false);
        assert!(!children.accept("alice", ours));
        assert!(children.is_empty());
        assert!(!children.has_room());
    }

    #[test]
    fn children_are_taken_up_to_the_cap() {
        let mut children = Children::new(true);
        // The child ends are kept alive so the links stay up; the parent
        // drops a child whose socket closes.
        let mut ends = Vec::new();
        for i in 0..MAX_CHILDREN {
            let (child, ours) = pair();
            ends.push(child);
            assert!(children.accept(&format!("user{i}"), ours));
        }
        assert_eq!(ends.len(), MAX_CHILDREN, "every link is still held");
        assert!(!children.has_room(), "a full parent has no room");

        let (_late, ours) = pair();
        assert!(!children.accept("one_too_many", ours));
        assert_eq!(children.len(), MAX_CHILDREN);
    }

    #[test]
    fn a_reconnecting_child_replaces_its_own_link() {
        let mut children = Children::new(true);
        let (_first, ours) = pair();
        assert!(children.accept("alice", ours));
        let (_second, again) = pair();
        assert!(children.accept("alice", again));
        assert_eq!(children.usernames(), vec!["alice".to_string()]);
    }

    #[test]
    fn a_search_reaches_every_child() {
        let mut children = Children::new(true);
        let (mut first, ours) = pair();
        let (mut second, theirs) = pair();
        children.accept("alice", ours);
        children.accept("bob", theirs);

        children.broadcast_search("seeker", 42, "jazz");

        let expected =
            distributed::build_search("seeker", 42, "jazz").get_buffer();
        assert_eq!(read_frames(&mut first, expected.len()), expected);
        assert_eq!(read_frames(&mut second, expected.len()), expected);
    }

    #[test]
    fn a_child_that_hung_up_loses_its_slot() {
        let mut children = Children::new(true);
        let (child, ours) = pair();
        children.accept("alice", ours);
        drop(child);

        // The first write after the peer is gone may still be buffered; the
        // second is the one that fails. Either way the child must not keep
        // its slot forever.
        for _ in 0..50 {
            children.broadcast_search("seeker", 1, "q");
            if children.is_empty() {
                break;
            }
        }
        assert!(children.is_empty(), "a dead child is dropped");
    }

    #[test]
    fn a_child_is_told_where_our_branch_sits() {
        let mut children = Children::new(true);
        let (mut child, ours) = pair();
        children.accept("alice", ours);

        children.send_stance_to("alice", "rooty", 3);

        let mut expected = distributed::build_branch_root("rooty").get_buffer();
        expected.extend(distributed::build_branch_level(3).get_buffer());
        assert_eq!(read_frames(&mut child, expected.len()), expected);
    }
}
