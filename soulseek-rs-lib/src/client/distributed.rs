//! Our place in the distributed search network, as a leaf: which parent we
//! hang from, which candidates we are still sounding out, and what to tell the
//! server when that changes.

use super::{Client, ClientOperation, Result, ServerMessage};
use crate::message::MessageReader;
use crate::message::distributed::{self, Distributed};
use crate::message::server::MessageFactory;
use crate::peer::ConnectionType;
use crate::utils::lock::RwLockExt;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// A candidate that has not answered the dial in this long is not a parent.
const LINK_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often an idle link looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_secs(1);
/// A parent relays the network's whole search stream; one that says nothing
/// for this long is gone, and holding its slot would block every later
/// candidate.
const LINK_IDLE: Duration = Duration::from_mins(1);
/// Moves of our branch are told to the server at most this often; a parent
/// that flaps its level must not make us flood the server.
const MOVE_INTERVAL: Duration = Duration::from_secs(1);
/// A distributed frame is a search or a branch fact; anything bigger is not
/// one of ours.
const MAX_FRAME: usize = 64 * 1024;
/// The server offers at most ten parents at a time; more candidates than
/// that in flight is a server that keeps offering, and each one is a thread.
const MAX_CANDIDATES: usize = 10;
/// Searches admitted from the parent per second. Every search costs a scan
/// of everything we share, and the tree carries the whole network's stream.
// ponytail: a flat budget; a word index over the shares is the upgrade path.
const SEARCH_BUDGET: u32 = 50;

/// Where we sit in the tree, as told to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub root: String,
    pub level: u32,
}

impl Branch {
    #[must_use]
    pub fn parentless(own: &str) -> Self {
        Self::under(own, 0)
    }

    #[must_use]
    pub fn under(root: &str, level: u32) -> Self {
        Self {
            root: root.to_string(),
            level,
        }
    }
}

struct Candidate {
    link: u64,
    level: Option<u32>,
    root: Option<String>,
    cancel: Arc<AtomicBool>,
}

/// A candidate to dial: the link number that tags everything it sends, and
/// the flag that tells its thread to stop.
pub struct Dial {
    pub link: u64,
    pub username: String,
    pub host: String,
    pub port: u16,
    pub cancel: Arc<AtomicBool>,
}

/// The leaf's view: at most one parent, adopted from the candidates the
/// server offered, the way Nicotine+ does it. Every method is a pure state
/// change so the rule is testable without a socket. Events carry the link
/// number they came from, so a stale thread cannot speak for a newer link
/// to the same user.
pub struct Leaf {
    own: String,
    parent: Option<String>,
    branch: Branch,
    candidates: HashMap<String, Candidate>,
    next_link: u64,
    searches: (Instant, u32),
    /// When we last told the server we moved, and whether a move since then
    /// is still waiting to be told.
    moved: Option<Instant>,
    move_pending: bool,
}

impl Leaf {
    #[must_use]
    pub fn new(own: &str) -> Self {
        Self {
            own: own.to_string(),
            parent: None,
            branch: Branch::parentless(own),
            candidates: HashMap::new(),
            next_link: 1,
            searches: (Instant::now(), 0),
            moved: None,
            move_pending: false,
        }
    }

    #[must_use]
    pub fn branch(&self) -> Branch {
        self.branch.clone()
    }

    /// Whether `link` is the live link to our parent.
    #[must_use]
    pub fn is_parent(&self, username: &str, link: u64) -> bool {
        self.parent.as_deref() == Some(username)
            && self.has_link(username, link)
    }

    /// New candidates to dial: those we are not already sounding out, up to
    /// the cap, and none at all while we hang from a parent.
    pub fn consider(
        &mut self,
        candidates: Vec<(String, String, u16)>,
    ) -> Vec<Dial> {
        if self.parent.is_some() {
            return Vec::new();
        }
        let mut dials = Vec::new();
        for (username, host, port) in candidates {
            if self.candidates.contains_key(&username)
                || self.candidates.len() >= MAX_CANDIDATES
            {
                continue;
            }
            let link = self.next_link;
            self.next_link += 1;
            let cancel = Arc::new(AtomicBool::new(false));
            self.candidates.insert(
                username.clone(),
                Candidate {
                    link,
                    level: None,
                    root: None,
                    cancel: cancel.clone(),
                },
            );
            dials.push(Dial {
                link,
                username,
                host,
                port,
                cancel,
            });
        }
        dials
    }

    /// A candidate said how deep it sits. A branch root does not always say
    /// so separately: level 0 is its own root. A negative level is nonsense
    /// and leaves the candidate unadoptable. When it is the parent talking,
    /// our own place moves with it and the new stance comes back.
    pub fn branch_level(
        &mut self,
        username: &str,
        link: u64,
        level: i32,
    ) -> Option<Branch> {
        if !self.has_link(username, link) {
            return None;
        }
        let candidate = self.candidates.get_mut(username)?;
        candidate.level = u32::try_from(level).ok();
        if level == 0 {
            candidate.root = Some(username.to_string());
        }
        self.moved_with_parent(username)
    }

    pub fn branch_root(
        &mut self,
        username: &str,
        link: u64,
        root: &str,
    ) -> Option<Branch> {
        if !self.has_link(username, link) {
            return None;
        }
        self.candidates.get_mut(username)?.root = Some(root.to_string());
        self.moved_with_parent(username)
    }

    /// A search came down from `username`. The first one from a candidate
    /// that has told us its place adopts it; the stance to announce comes back.
    pub fn search_from(&mut self, username: &str, link: u64) -> Option<Branch> {
        if self.parent.is_some() || !self.has_link(username, link) {
            return None;
        }
        let candidate = self.candidates.get(username)?;
        let (Some(level), Some(root)) =
            (candidate.level, candidate.root.clone())
        else {
            return None;
        };
        let parent = self.candidates.remove(username)?;
        self.cancel_candidates();
        self.candidates.insert(username.to_string(), parent);
        self.parent = Some(username.to_string());
        self.branch = Branch::under(&root, level.saturating_add(1));
        Some(self.branch.clone())
    }

    /// Whether to answer one more search from the parent now: at most
    /// `SEARCH_BUDGET` a second, the rest dropped.
    pub fn admit_search(&mut self, now: Instant) -> bool {
        if now.duration_since(self.searches.0) >= Duration::from_secs(1) {
            self.searches = (now, 0);
        }
        self.searches.1 += 1;
        self.searches.1 <= SEARCH_BUDGET
    }

    /// The link to `username` closed. True when that was the parent, which
    /// puts us back to parentless.
    pub fn closed(&mut self, username: &str, link: u64) -> bool {
        if !self.has_link(username, link) {
            return false;
        }
        self.candidates.remove(username);
        if self.parent.as_deref() != Some(username) {
            return false;
        }
        self.parent = None;
        self.branch = Branch::parentless(&self.own);
        true
    }

    pub fn reset(&mut self) {
        self.cancel_candidates();
        self.parent = None;
        self.branch = Branch::parentless(&self.own);
    }

    fn has_link(&self, username: &str, link: u64) -> bool {
        self.candidates
            .get(username)
            .is_some_and(|c| c.link == link)
    }

    fn moved_with_parent(&mut self, username: &str) -> Option<Branch> {
        if self.parent.as_deref() != Some(username) {
            return None;
        }
        let candidate = self.candidates.get(username)?;
        let branch = Branch::under(
            candidate.root.as_deref()?,
            candidate.level?.saturating_add(1),
        );
        if branch == self.branch {
            return None;
        }
        self.branch = branch;
        self.move_pending = true;
        self.due_announcement(Instant::now())
    }

    /// The branch to tell the server about, if a move is waiting and the
    /// last one was told at least [`MOVE_INTERVAL`] ago.
    pub fn due_announcement(&mut self, now: Instant) -> Option<Branch> {
        if !self.move_pending
            || self
                .moved
                .is_some_and(|at| now.duration_since(at) < MOVE_INTERVAL)
        {
            return None;
        }
        self.move_pending = false;
        self.moved = Some(now);
        Some(self.branch.clone())
    }

    fn cancel_candidates(&mut self) {
        for candidate in self.candidates.values() {
            candidate.cancel.store(true, Ordering::Relaxed);
        }
        self.candidates.clear();
    }
}

/// Tell the server where we sit now.
pub fn announce(
    server: Option<&Sender<ServerMessage>>,
    branch: &Branch,
    has_parent: bool,
    accept_children: bool,
) {
    let Some(server) = server else {
        return;
    };
    for message in distributed::stance(
        &branch.root,
        branch.level,
        has_parent,
        accept_children,
    ) {
        let _ = server.send(ServerMessage::SendMessage(message));
    }
}

/// Tell the server where we sit, and tell our children too: a branch that
/// moved leaves every child reporting a stale place until they hear it.
pub fn announce_move(
    ctx: &mut super::ClientContext,
    branch: &Branch,
    has_parent: bool,
) {
    let server = ctx.server_sender.clone();
    let accepting = ctx.children.accepting();
    announce(server.as_ref(), branch, has_parent, accepting);
    ctx.children.broadcast_stance(&branch.root, branch.level);
}

/// Dial a parent candidate on its own thread: a `D` PeerInit, then whatever it
/// sends comes back as operations until the link closes or is cancelled.
pub fn spawn_link(dial: Dial, own: String, ops: Sender<ClientOperation>) {
    let closed = ClientOperation::ParentClosed {
        parent: dial.username.clone(),
        link: dial.link,
    };
    let thread_ops = ops.clone();
    let spawned = std::thread::Builder::new()
        .name("soulseek-parent".to_string())
        .spawn(move || {
            if let Err(e) = link(&dial, &own, &thread_ops)
                && !dial.cancel.load(Ordering::Relaxed)
            {
                crate::debug!(
                    "[distributed] link to {} ended: {}",
                    dial.username,
                    e
                );
                let _ = thread_ops.send(ClientOperation::ParentClosed {
                    parent: dial.username,
                    link: dial.link,
                });
            }
        });
    if spawned.is_err() {
        let _ = ops.send(closed);
    }
}

fn link(
    dial: &Dial,
    own: &str,
    ops: &Sender<ClientOperation>,
) -> io::Result<()> {
    let addr = (dial.host.as_str(), dial.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "no address")
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, LINK_CONNECT_TIMEOUT)?;
    if dial.cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    stream.set_read_timeout(Some(CANCEL_POLL))?;
    stream.write_all(
        &MessageFactory::build_peer_init_message(own, ConnectionType::D, 0)
            .get_buffer(),
    )?;
    let mut reader = MessageReader::new();
    let mut heard = Instant::now();
    loop {
        if dial.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match reader.read_from_socket(&mut stream) {
            Ok(()) => heard = Instant::now(),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if heard.elapsed() > LINK_IDLE {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "silent parent",
                    ));
                }
                continue;
            }
            Err(e) => return Err(e),
        }
        while let Some(mut frame) = reader.extract_message()? {
            let Some(operation) = operation_of(dial, &mut frame) else {
                continue;
            };
            if ops.send(operation).is_err() {
                return Ok(());
            }
        }
        if reader.buffer_len() > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized frame",
            ));
        }
    }
}

fn operation_of(
    dial: &Dial,
    frame: &mut crate::message::Message,
) -> Option<ClientOperation> {
    let parent = dial.username.clone();
    let link = dial.link;
    Some(match distributed::parse(frame)? {
        Distributed::BranchLevel(level) => ClientOperation::ParentBranchLevel {
            parent,
            link,
            level,
        },
        Distributed::BranchRoot(root) => {
            ClientOperation::ParentBranchRoot { parent, link, root }
        }
        Distributed::Search {
            username,
            token,
            query,
        } => ClientOperation::ParentSearch {
            parent,
            link,
            username,
            token,
            query,
        },
    })
}

impl Client {
    /// Offer parent candidates as the server does with PossibleParents:
    /// `(username, host, port)`. The first candidate to send a search after
    /// telling us its branch becomes our parent.
    ///
    /// # Errors
    /// Returns [`crate::SoulseekRs::NotConnected`] before [`Client::connect`].
    pub fn consider_parents(
        &self,
        candidates: Vec<(String, String, u16)>,
    ) -> Result<()> {
        let ops = self
            .context
            .read_safe()?
            .operations
            .clone()
            .ok_or(crate::SoulseekRs::NotConnected)?;
        ops.send(ClientOperation::PossibleParents(candidates))
            .map_err(|_| crate::SoulseekRs::NotConnected)
    }

    /// Where we sit in the distributed search network: the branch root's name
    /// and our level, which is ourselves at 0 while we have no parent.
    #[must_use]
    pub fn distributed_branch(&self) -> (String, u32) {
        self.context.read_safe().map_or_else(
            |_| (self.username.clone(), 0),
            |ctx| {
                let branch = ctx.leaf.branch();
                (branch.root, branch.level)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf() -> Leaf {
        Leaf::new("me")
    }

    fn candidate(name: &str) -> (String, String, u16) {
        (name.to_string(), "127.0.0.1".to_string(), 2234)
    }

    /// Dial the candidates and return their link numbers by name.
    fn dial(leaf: &mut Leaf, names: &[&str]) -> HashMap<String, Dial> {
        leaf.consider(names.iter().map(|n| candidate(n)).collect())
            .into_iter()
            .map(|d| (d.username.clone(), d))
            .collect()
    }

    #[test]
    fn a_leaf_without_a_parent_is_its_own_root_at_level_zero() {
        assert_eq!(leaf().branch(), Branch::parentless("me"));
    }

    #[test]
    fn candidates_are_dialled_only_while_there_is_no_parent() {
        let mut leaf = leaf();
        let dials = dial(&mut leaf, &["alice", "bob"]);
        assert_eq!(dials.len(), 2);
        let alice = dials["alice"].link;
        leaf.branch_level("alice", alice, 1);
        leaf.branch_root("alice", alice, "rooty");
        assert!(
            leaf.search_from("alice", alice).is_some(),
            "alice becomes the parent"
        );
        assert!(leaf.consider(vec![candidate("carol")]).is_empty());
    }

    #[test]
    fn the_first_search_from_a_candidate_with_branch_info_adopts_it() {
        let mut leaf = leaf();
        let dials = dial(&mut leaf, &["alice", "bob"]);
        let (alice, bob) = (dials["alice"].link, dials["bob"].link);
        assert_eq!(leaf.search_from("alice", alice), None, "no branch yet");
        leaf.branch_level("alice", alice, 3);
        assert_eq!(leaf.search_from("alice", alice), None, "level, no root");
        leaf.branch_root("alice", alice, "rooty");
        assert_eq!(
            leaf.search_from("alice", alice),
            Some(Branch::under("rooty", 4)),
            "level 3 parent puts us at level 4"
        );
        assert_eq!(leaf.branch(), Branch::under("rooty", 4));
        assert!(leaf.is_parent("alice", alice));
        assert_eq!(leaf.search_from("alice", alice), None, "already parent");
        assert_eq!(leaf.search_from("bob", bob), None, "a loser is ignored");
        assert!(
            dials["bob"].cancel.load(Ordering::Relaxed),
            "the losing link is told to stop"
        );
        assert!(!dials["alice"].cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn a_level_zero_candidate_is_its_own_root() {
        let mut leaf = leaf();
        let alice = dial(&mut leaf, &["alice"])["alice"].link;
        leaf.branch_level("alice", alice, 0);
        assert_eq!(
            leaf.search_from("alice", alice),
            Some(Branch::under("alice", 1))
        );
    }

    #[test]
    fn a_hostile_branch_level_neither_panics_nor_adopts() {
        let mut leaf = leaf();
        let alice = dial(&mut leaf, &["alice"])["alice"].link;
        leaf.branch_root("alice", alice, "rooty");
        leaf.branch_level("alice", alice, -1);
        assert_eq!(leaf.search_from("alice", alice), None, "negative level");
        leaf.branch_level("alice", alice, i32::MAX);
        assert_eq!(
            leaf.search_from("alice", alice),
            Some(Branch::under("rooty", 1 << 31))
        );
    }

    #[test]
    fn the_parents_later_moves_move_us_too_but_are_told_once_a_second() {
        let mut leaf = leaf();
        let alice = dial(&mut leaf, &["alice"])["alice"].link;
        leaf.branch_level("alice", alice, 0);
        leaf.search_from("alice", alice);
        assert_eq!(leaf.branch_level("alice", alice, 0), None, "unchanged");
        assert_eq!(
            leaf.branch_root("alice", alice, "grand"),
            Some(Branch::under("grand", 1))
        );
        assert_eq!(
            leaf.branch_level("alice", alice, 2),
            None,
            "a second move within the second waits"
        );
        assert_eq!(leaf.branch(), Branch::under("grand", 3), "but it took");
        assert_eq!(leaf.due_announcement(Instant::now()), None);
        assert_eq!(
            leaf.due_announcement(Instant::now() + MOVE_INTERVAL),
            Some(Branch::under("grand", 3))
        );
        assert_eq!(
            leaf.due_announcement(Instant::now() + MOVE_INTERVAL),
            None,
            "told once"
        );
    }

    #[test]
    fn a_parent_that_becomes_a_root_is_the_root_from_then_on() {
        let mut leaf = leaf();
        let alice = dial(&mut leaf, &["alice"])["alice"].link;
        leaf.branch_root("alice", alice, "grand");
        leaf.branch_level("alice", alice, 5);
        assert_eq!(
            leaf.search_from("alice", alice),
            Some(Branch::under("grand", 6))
        );
        assert_eq!(
            leaf.branch_level("alice", alice, 0),
            Some(Branch::under("alice", 1))
        );
    }

    #[test]
    fn a_stale_link_cannot_speak_for_a_newer_one() {
        let mut leaf = leaf();
        let old = dial(&mut leaf, &["alice"])["alice"].link;
        assert!(!leaf.closed("alice", old + 7), "unknown link is ignored");
        leaf.closed("alice", old);
        let new = dial(&mut leaf, &["alice"])["alice"].link;
        assert_ne!(old, new);
        leaf.branch_level("alice", old, 0);
        assert_eq!(leaf.search_from("alice", old), None, "old link ignored");
        assert!(
            !leaf.closed("alice", old),
            "a late close leaves the new one"
        );
        leaf.branch_level("alice", new, 0);
        assert!(leaf.search_from("alice", new).is_some());
    }

    #[test]
    fn a_later_batch_keeps_the_candidates_in_flight_and_stays_under_the_cap() {
        let mut leaf = leaf();
        let first = dial(&mut leaf, &["alice"])["alice"].link;
        let again = dial(&mut leaf, &["alice", "bob"]);
        assert!(
            !again.contains_key("alice"),
            "alice is already being dialled"
        );
        assert!(again.contains_key("bob"));
        assert!(leaf.has_link("alice", first));
        let names: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
        let more = leaf
            .consider(names.iter().map(|n| candidate(n)).collect::<Vec<_>>());
        assert_eq!(more.len(), MAX_CANDIDATES - 2);
    }

    #[test]
    fn losing_the_parent_or_a_reset_returns_to_parentless() {
        let mut leaf = leaf();
        let dials = dial(&mut leaf, &["alice", "bob"]);
        let alice = dials["alice"].link;
        leaf.branch_level("alice", alice, 0);
        leaf.search_from("alice", alice);
        assert!(!leaf.closed("bob", dials["bob"].link), "not the parent");
        assert!(leaf.closed("alice", alice));
        assert_eq!(leaf.branch(), Branch::parentless("me"));

        let dials = dial(&mut leaf, &["alice", "carol"]);
        let alice = dials["alice"].link;
        leaf.branch_level("alice", alice, 0);
        leaf.search_from("alice", alice);
        leaf.reset();
        assert_eq!(leaf.branch(), Branch::parentless("me"));
        assert!(dials["alice"].cancel.load(Ordering::Relaxed));
        assert!(dials["carol"].cancel.load(Ordering::Relaxed));
        assert!(!leaf.consider(vec![candidate("bob")]).is_empty());
    }

    #[test]
    fn searches_beyond_the_budget_are_dropped_until_the_next_second() {
        let mut leaf = leaf();
        let now = Instant::now();
        for _ in 0..SEARCH_BUDGET {
            assert!(leaf.admit_search(now));
        }
        assert!(!leaf.admit_search(now));
        assert!(leaf.admit_search(now + Duration::from_secs(1)));
    }

    #[test]
    fn announcing_tells_the_server_the_root_the_level_and_that_we_have_a_parent()
     {
        let (tx, rx) = std::sync::mpsc::channel();
        announce(Some(&tx), &Branch::under("rooty", 4), true, false);
        let mut sent: Vec<crate::message::Message> = Vec::new();
        while let Ok(ServerMessage::SendMessage(m)) = rx.try_recv() {
            sent.push(m);
        }
        let codes: Vec<u32> = sent
            .iter()
            .map(|m| u32::from_le_bytes(m.get_data()[..4].try_into().unwrap()))
            .collect();
        assert_eq!(codes, vec![71, 127, 126, 100]);
        assert_eq!(sent[0].get_data()[4], 0, "HaveNoParent is false");
        let mut root = sent[1].clone();
        root.set_pointer(4);
        assert_eq!(root.read_string(), "rooty");
        assert_eq!(&sent[2].get_data()[4..8], &4u32.to_le_bytes());
        assert_eq!(sent[3].get_data()[4], 0, "AcceptChildren is false");
    }
}
