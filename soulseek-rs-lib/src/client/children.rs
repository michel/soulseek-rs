//! Serving children: the other half of the distributed search network.
//!
//! A leaf only reads the tree; a parent also carries it. When the client is
//! configured to accept children, peers dial us with a `D` connection, we tell
//! each one where our branch sits, and every search that reaches us is passed
//! down to all of them. That is the whole duty — a child answers searches
//! itself, and reports its own place to the server.
//!
//! How many children to carry follows Nicotine+: the server announces the
//! upload speed a parent needs (`ParentMinSpeed`, code 83) and the divisor
//! turning speed into a child count (`ParentSpeedRatio`, code 84), and a
//! client whose recorded speed is below the minimum carries none. Until the
//! server has reported our own speed there is nothing to apply the formula to,
//! so the configured cap stands — which is how slskd runs the whole time, with
//! an operator-set limit and no speed gate.
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

/// How many children to carry at once. Nicotine+ caps its speed-derived limit
/// here too, for the same reason: each child is a socket and a copy of the
/// network's whole search stream.
pub const MAX_CHILDREN: usize = 10;

/// The child limit the server's figures imply, following Nicotine+:
/// `speed / ratio / 100`, capped, and none at all when our recorded speed is
/// below the minimum the server set.
///
/// `None` means the server's figures do not decide it — either it has not
/// reported our speed yet, or it never announced a ratio at all (soulfind
/// does not) — and the caller's own cap stands, which is how slskd runs the
/// whole time.
#[must_use]
pub fn limit_from_speed(
    own_speed: Option<u32>,
    min_speed: u32,
    ratio: u32,
) -> Option<usize> {
    if ratio == 0 {
        return None;
    }
    let speed = own_speed?;
    if speed < min_speed {
        return Some(0);
    }
    Some((speed / ratio / 100) as usize)
}

/// The children hanging from us, keyed by username.
pub struct Children {
    links: HashMap<String, TcpStream>,
    /// The cap in force: [`MAX_CHILDREN`], lowered to what the server's
    /// figures allow once it has reported our speed.
    max: usize,
    accepting: bool,
    /// Whether anything is feeding us the search stream — a parent of our own,
    /// or the server. Nicotine+ refuses children without one, and rightly:
    /// a child hanging from a branch that receives nothing gets nothing.
    fed: bool,
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
            fed: false,
        }
    }

    /// Apply the server's figures: `own_speed` is what it records for us, and
    /// the other two are what it announced in codes 83 and 84.
    pub fn set_limits(
        &mut self,
        own_speed: Option<u32>,
        min_speed: u32,
        ratio: u32,
    ) {
        self.max = limit_from_speed(own_speed, min_speed, ratio)
            .map_or(MAX_CHILDREN, |limit| limit.min(MAX_CHILDREN));
        debug!("[distributed] child limit is now {}", self.max);
    }

    /// Say whether something is feeding us the search stream. A client with
    /// nothing to relay takes no children, and drops any it has.
    pub const fn set_fed(&mut self, fed: bool) {
        self.fed = fed;
    }

    #[must_use]
    pub const fn is_fed(&self) -> bool {
        self.fed
    }

    /// Whether we would take another child right now: we serve children, we
    /// have something to relay, and we are under the cap. This is what the
    /// server is told in `AcceptChildren` (code 100).
    #[must_use]
    pub fn has_room(&self) -> bool {
        self.accepting && self.fed && self.links.len() < self.max
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

    /// Take on `username` as a child. Refused when we serve no children, have
    /// nothing to relay, are full, or already carry that user.
    pub fn accept(&mut self, username: &str, stream: TcpStream) -> bool {
        if !self.has_room() {
            return false;
        }
        // A second connection from a child we already carry is refused, as
        // Nicotine+ does: the link we hold is the live one, and dropping it
        // for an unproven second would cost that child its search stream.
        if self.links.contains_key(username) {
            debug!(
                "[distributed] refusing child {}: already carried",
                username
            );
            return false;
        }
        self.links.insert(username.to_string(), stream);
        true
    }

    /// Drop every child, closing their links: what a client does when it can
    /// no longer feed them.
    pub fn drop_all(&mut self) {
        if !self.links.is_empty() {
            debug!("[distributed] dropping {} children", self.links.len());
        }
        self.links.clear();
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
        if self.is_empty() {
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

    /// A registry that serves children and has something to relay.
    fn serving() -> Children {
        let mut children = Children::new(true);
        children.set_fed(true);
        children
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
        children.set_fed(true);
        assert!(!children.accept("alice", ours));
        assert!(children.is_empty());
        assert!(!children.has_room());
    }

    #[test]
    fn a_client_nothing_feeds_takes_no_children() {
        // Nicotine+ refuses children while it has neither a parent nor the
        // server relaying searches: a child would hang from a branch that
        // receives nothing.
        let (_child, ours) = pair();
        let mut children = Children::new(true);
        assert!(!children.has_room(), "unfed means no room");
        assert!(!children.accept("alice", ours));

        children.set_fed(true);
        let (_second, again) = pair();
        assert!(children.accept("alice", again));
    }

    #[test]
    fn children_are_let_go_when_the_stream_dries_up() {
        let mut children = serving();
        let (_child, ours) = pair();
        children.accept("alice", ours);

        children.set_fed(false);
        children.drop_all();
        assert!(children.is_empty());
        assert!(!children.has_room());
    }

    #[test]
    fn children_are_taken_up_to_the_cap() {
        let mut children = serving();
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
    fn a_second_link_from_a_child_we_carry_is_refused() {
        // The link we hold is the live one; dropping it for an unproven
        // second would cost that child its search stream.
        let mut children = serving();
        let (_first, ours) = pair();
        assert!(children.accept("alice", ours));
        let (_second, again) = pair();
        assert!(!children.accept("alice", again));
        assert_eq!(children.usernames(), vec!["alice".to_string()]);
    }

    #[test]
    fn the_child_limit_follows_the_servers_figures() {
        // Nicotine+: speed / ratio / 100, capped, and none at all below the
        // minimum speed the server set.
        assert_eq!(limit_from_speed(Some(50_000), 1_024, 50), Some(10));
        assert_eq!(limit_from_speed(Some(10_000), 1_024, 50), Some(2));
        assert_eq!(
            limit_from_speed(Some(500), 1_024, 50),
            Some(0),
            "too slow to parent"
        );
        assert_eq!(
            limit_from_speed(Some(10_000), 1_024, 0),
            None,
            "a server that never announced a ratio does not decide the limit"
        );
        assert_eq!(
            limit_from_speed(None, 1_024, 50),
            None,
            "an unreported speed leaves the caller's own cap standing"
        );
    }

    #[test]
    fn a_speed_the_server_has_not_reported_leaves_the_cap_alone() {
        let mut children = serving();
        children.set_limits(None, 1_024, 50);
        assert!(children.has_room(), "an unreported speed leaves us open");

        // The server's figures win: at this speed the formula allows two.
        children.set_limits(Some(10_000), 1_024, 50);
        let mut ends = Vec::new();
        for i in 0..2 {
            let (child, ours) = pair();
            ends.push(child);
            assert!(children.accept(&format!("user{i}"), ours));
        }
        assert!(!children.has_room(), "two is the limit at this speed");

        children.set_limits(Some(100), 1_024, 50);
        assert!(!children.has_room(), "a slow client carries none");

        children.set_limits(Some(100), 1_024, 0);
        assert!(
            children.has_room(),
            "a server with no figures leaves our own cap standing"
        );
        assert_eq!(ends.len(), 2);
    }

    #[test]
    fn a_search_reaches_every_child() {
        let mut children = serving();
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
        let mut children = serving();
        let (child, ours) = pair();
        children.accept("alice", ours);
        drop(child);

        // Writes keep landing in the send buffer until the peer's RST has
        // crossed the loopback, which under load takes real wall-clock time.
        for _ in 0..100 {
            children.broadcast_search("seeker", 1, "q");
            if children.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(children.is_empty(), "a dead child is dropped");
    }

    #[test]
    fn a_child_is_told_where_our_branch_sits() {
        let mut children = serving();
        let (mut child, ours) = pair();
        children.accept("alice", ours);

        children.send_stance_to("alice", "rooty", 3);

        let mut expected = distributed::build_branch_root("rooty").get_buffer();
        expected.extend(distributed::build_branch_level(3).get_buffer());
        assert_eq!(read_frames(&mut child, expected.len()), expected);
    }
}
