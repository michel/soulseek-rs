//! Our place in the distributed search network, as a leaf: which parent we
//! hang from, which candidates we are still sounding out, and what to tell the
//! server when that changes.

use super::{Client, ClientOperation, Result, ServerMessage};
use crate::message::Message;
use crate::message::distributed::{self, Distributed};
use crate::message::server::MessageFactory;
use crate::peer::ConnectionType;
use crate::utils::lock::RwLockExt;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// A candidate that has not answered the dial in this long is not a parent.
const LINK_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often an idle link looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_secs(1);
/// A distributed frame is a search or a branch fact; anything bigger is not
/// one of ours.
const MAX_FRAME: usize = 64 * 1024;

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

#[derive(Default)]
struct Candidate {
    level: Option<i32>,
    root: Option<String>,
    cancel: Arc<AtomicBool>,
}

/// A candidate to dial, and the flag that tells its link to stop.
pub struct Dial {
    pub username: String,
    pub host: String,
    pub port: u16,
    pub cancel: Arc<AtomicBool>,
}

/// The leaf's view: at most one parent, adopted from the candidates the
/// server offered, the way Nicotine+ does it. Every method is a pure state
/// change so the rule is testable without a socket.
pub struct Leaf {
    own: String,
    parent: Option<String>,
    branch: Branch,
    candidates: HashMap<String, Candidate>,
}

impl Leaf {
    #[must_use]
    pub fn new(own: &str) -> Self {
        Self {
            own: own.to_string(),
            parent: None,
            branch: Branch::parentless(own),
            candidates: HashMap::new(),
        }
    }

    #[must_use]
    pub fn branch(&self) -> Branch {
        self.branch.clone()
    }

    #[must_use]
    pub fn is_parent(&self, username: &str) -> bool {
        self.parent.as_deref() == Some(username)
    }

    /// New candidates: dial them all, unless we already hang from a parent.
    /// Candidates still being sounded out from an earlier batch are dropped.
    pub fn consider(
        &mut self,
        candidates: Vec<(String, String, u16)>,
    ) -> Vec<Dial> {
        if self.parent.is_some() {
            return Vec::new();
        }
        self.cancel_candidates();
        candidates
            .into_iter()
            .map(|(username, host, port)| {
                let cancel = Arc::new(AtomicBool::new(false));
                self.candidates.insert(
                    username.clone(),
                    Candidate {
                        cancel: cancel.clone(),
                        ..Candidate::default()
                    },
                );
                Dial {
                    username,
                    host,
                    port,
                    cancel,
                }
            })
            .collect()
    }

    /// A branch root does not always say so separately: level 0 is its own
    /// root.
    pub fn branch_level(&mut self, username: &str, level: i32) {
        let Some(candidate) = self.candidates.get_mut(username) else {
            return;
        };
        candidate.level = Some(level);
        if level == 0 && candidate.root.is_none() {
            candidate.root = Some(username.to_string());
        }
    }

    pub fn branch_root(&mut self, username: &str, root: &str) {
        if let Some(candidate) = self.candidates.get_mut(username) {
            candidate.root = Some(root.to_string());
        }
    }

    /// A search came down from `username`. The first one from a candidate that
    /// has told us its place adopts it; the stance to announce comes back.
    pub fn search_from(&mut self, username: &str) -> Option<Branch> {
        if self.parent.is_some() {
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
        self.branch =
            Branch::under(&root, u32::try_from(level + 1).unwrap_or(1));
        Some(self.branch.clone())
    }

    /// The link to `username` closed. True when that was the parent, which
    /// puts us back to parentless.
    pub fn closed(&mut self, username: &str) -> bool {
        self.candidates.remove(username);
        if !self.is_parent(username) {
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
) {
    let Some(server) = server else {
        return;
    };
    for message in distributed::stance(&branch.root, branch.level, has_parent) {
        let _ = server.send(ServerMessage::SendMessage(message));
    }
}

/// Dial a parent candidate on its own thread: a `D` PeerInit, then whatever it
/// sends comes back as operations until the link closes or is cancelled.
pub fn spawn_link(dial: Dial, own: String, ops: Sender<ClientOperation>) {
    let _ = std::thread::Builder::new()
        .name("soulseek-parent".to_string())
        .spawn(move || {
            let parent = dial.username.clone();
            if let Err(e) = link(&dial, &own, &ops)
                && !dial.cancel.load(Ordering::Relaxed)
            {
                crate::debug!("[distributed] link to {} ended: {}", parent, e);
                let _ = ops.send(ClientOperation::ParentClosed { parent });
            }
        });
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
    stream.set_read_timeout(Some(CANCEL_POLL))?;
    stream.write_all(
        &MessageFactory::build_peer_init_message(own, ConnectionType::D, 0)
            .get_buffer(),
    )?;
    loop {
        if dial.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(mut frame) = read_frame(&mut stream)? else {
            continue;
        };
        let parent = dial.username.clone();
        let operation = match distributed::parse(&mut frame) {
            Some(Distributed::BranchLevel(level)) => {
                ClientOperation::ParentBranchLevel { parent, level }
            }
            Some(Distributed::BranchRoot(root)) => {
                ClientOperation::ParentBranchRoot { parent, root }
            }
            Some(Distributed::Search {
                username,
                token,
                query,
            }) => ClientOperation::ParentSearch {
                parent,
                username,
                token,
                query,
            },
            None => continue,
        };
        if ops.send(operation).is_err() {
            return Ok(());
        }
    }
}

/// One framed message, or `None` when the read timed out with nothing sent.
fn read_frame(stream: &mut TcpStream) -> io::Result<Option<Message>> {
    let mut len = [0u8; 4];
    match stream.read_exact(&mut len) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e),
    }
    let size = u32::from_le_bytes(len) as usize;
    if size > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized frame",
        ));
    }
    let mut data = len.to_vec();
    data.resize(4 + size, 0);
    stream.read_exact(&mut data[4..])?;
    Ok(Some(Message::new_with_data(data)))
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

    #[test]
    fn a_leaf_without_a_parent_is_its_own_root_at_level_zero() {
        assert_eq!(leaf().branch(), Branch::parentless("me"));
    }

    #[test]
    fn candidates_are_dialled_only_while_there_is_no_parent() {
        let mut leaf = leaf();
        let dial = leaf.consider(vec![candidate("alice"), candidate("bob")]);
        assert_eq!(dial.len(), 2);
        leaf.branch_level("alice", 1);
        leaf.branch_root("alice", "rooty");
        assert!(
            leaf.search_from("alice").is_some(),
            "alice becomes the parent"
        );
        assert!(leaf.consider(vec![candidate("carol")]).is_empty());
    }

    #[test]
    fn the_first_search_from_a_candidate_with_branch_info_adopts_it() {
        let mut leaf = leaf();
        leaf.consider(vec![candidate("alice"), candidate("bob")]);
        assert_eq!(leaf.search_from("alice"), None, "no branch info yet");
        leaf.branch_level("alice", 3);
        assert_eq!(leaf.search_from("alice"), None, "level without root");
        leaf.branch_root("alice", "rooty");
        assert_eq!(
            leaf.search_from("alice"),
            Some(Branch::under("rooty", 4)),
            "level 3 parent puts us at level 4"
        );
        assert_eq!(leaf.branch(), Branch::under("rooty", 4));
        assert_eq!(leaf.search_from("alice"), None, "already the parent");
        assert_eq!(leaf.search_from("bob"), None, "a loser is ignored");
    }

    #[test]
    fn a_level_zero_candidate_is_its_own_root() {
        let mut leaf = leaf();
        leaf.consider(vec![candidate("alice")]);
        leaf.branch_level("alice", 0);
        assert_eq!(leaf.search_from("alice"), Some(Branch::under("alice", 1)));
    }

    #[test]
    fn losing_the_parent_or_a_reset_returns_to_parentless() {
        let mut leaf = leaf();
        leaf.consider(vec![candidate("alice")]);
        leaf.branch_level("alice", 0);
        leaf.search_from("alice");
        assert!(!leaf.closed("bob"), "a stranger closing changes nothing");
        assert!(leaf.closed("alice"));
        assert_eq!(leaf.branch(), Branch::parentless("me"));

        leaf.consider(vec![candidate("alice")]);
        leaf.branch_level("alice", 0);
        leaf.search_from("alice");
        leaf.reset();
        assert_eq!(leaf.branch(), Branch::parentless("me"));
        assert!(!leaf.consider(vec![candidate("bob")]).is_empty());
    }

    fn candidate(name: &str) -> (String, String, u16) {
        (name.to_string(), "127.0.0.1".to_string(), 2234)
    }
}
