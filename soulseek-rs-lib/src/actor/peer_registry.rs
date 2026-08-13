use crate::actor::peer_actor::{PeerActor, PeerMessage};
use crate::actor::{ActorHandle, ActorSystem};
use crate::client::ClientOperation;
use crate::message::MessageReader;
use crate::peer::Peer;
use crate::utils::lock::MutexExt;
use crate::{debug, error};

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Source of unique per-actor ids so terminal-outcome eviction can be made
/// identity-aware (a replaced actor must not evict its replacement).
static NEXT_PEER_ID: AtomicU64 = AtomicU64::new(1);

/// Registered peers keyed by username, each stored with the unique id of the
/// actor currently occupying the slot.
type PeerMap = HashMap<String, (u64, ActorHandle<PeerMessage>, Instant)>;

pub struct PeerRegistry {
    peers: Arc<Mutex<PeerMap>>,
    actor_system: Arc<ActorSystem>,
    client_channel: Sender<ClientOperation>,
    own_username: String,
    capacity: Arc<AtomicUsize>,
}

impl PeerRegistry {
    #[must_use]
    pub fn new(
        actor_system: Arc<ActorSystem>,
        client_channel: Sender<ClientOperation>,
        own_username: String,
    ) -> Self {
        Self::with_capacity(
            actor_system,
            client_channel,
            own_username,
            Arc::new(AtomicUsize::new(usize::MAX)),
        )
    }

    #[must_use]
    pub(crate) fn with_capacity(
        actor_system: Arc<ActorSystem>,
        client_channel: Sender<ClientOperation>,
        own_username: String,
        capacity: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            actor_system,
            client_channel,
            own_username,
            capacity,
        }
    }

    pub fn register_peer(
        &self,
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
    ) -> Result<ActorHandle<PeerMessage>, String> {
        self.register_peer_protected(peer, stream, reader, &HashSet::new())
    }

    pub(crate) fn register_peer_protected(
        &self,
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
        protected: &HashSet<String>,
    ) -> Result<ActorHandle<PeerMessage>, String> {
        let username = peer.username.clone();
        let id = NEXT_PEER_ID.fetch_add(1, Ordering::Relaxed);

        let actor = PeerActor::new(
            peer,
            stream,
            reader,
            self.client_channel.clone(),
            self.own_username.clone(),
            id,
        );

        // Take the map lock before the actor exists: a peer that dies
        // instantly (refused dial, immediate hangup) reports its terminal
        // outcome to the client loop, whose eviction takes this same lock.
        // With the insert racing the spawn, eviction could run first, find
        // nothing, and the entry inserted afterwards became a permanent
        // zombie claiming the username.
        let mut peers = self
            .peers
            .lock_safe()
            .map_err(|e| format!("peer registry lock poisoned: {e}"))?;

        let capacity = self.capacity.load(Ordering::Relaxed).max(1);
        if peers.len() >= capacity && !peers.contains_key(&username) {
            let victim = peers
                .iter()
                .filter(|(name, _)| !protected.contains(name.as_str()))
                .min_by_key(|(_, (_, _, registered))| *registered)
                .map(|(name, _)| name.clone());
            let Some(victim) = victim else {
                return Err(format!(
                    "peer registry at capacity ({capacity}) and every peer \
                     is busy; refusing {username}"
                ));
            };
            if let Some((_, handle, _)) = peers.remove(&victim) {
                let _ = handle.stop();
                debug!(
                    "[peer_registry] evicted idle peer {} to admit {}",
                    victim, username
                );
            }
        }

        let handle = self
            .actor_system
            .try_spawn_with_handle(actor, |actor, handle| {
                actor.set_self_handle(handle);
            })
            .map_err(|e| format!("failed to spawn peer actor thread: {e}"))?;
        // Stop any actor already registered under this username so it does not
        // become an orphan pinning a pool worker forever. Eviction on the
        // replaced actor's later shutdown is identity-aware (keyed on its id),
        // so stopping it here cannot evict this new connection.
        if let Some((_, old_handle, _)) =
            peers.insert(username.clone(), (id, handle.clone(), Instant::now()))
        {
            let _ = old_handle.stop();
            debug!(
                "[peer_registry] Replaced existing peer actor for {}",
                username
            );
        }

        Ok(handle)
    }

    #[must_use]
    pub fn get_peer(&self, username: &str) -> Option<ActorHandle<PeerMessage>> {
        match self.peers.lock_safe() {
            Ok(peers) => {
                peers.get(username).map(|(_, handle, _)| handle.clone())
            }
            Err(e) => {
                error!("[peer_registry] get_peer: {}", e);
                None
            }
        }
    }

    #[must_use]
    pub fn remove_peer(
        &self,
        username: &str,
    ) -> Option<ActorHandle<PeerMessage>> {
        let mut peers = match self.peers.lock_safe() {
            Ok(p) => p,
            Err(e) => {
                error!("[peer_registry] remove_peer: {}", e);
                return None;
            }
        };
        let removed = peers.remove(username);

        if removed.is_some() {
            debug!("[peer_registry] Removed peer actor for {}", username);
        }

        removed.map(|(_, handle, _)| handle)
    }

    /// Remove and return the actor for `username` only if it is still the actor
    /// with `id`. A stale (replaced) actor's terminal notification therefore
    /// cannot evict the newer actor that now occupies the slot.
    #[must_use]
    pub fn remove_peer_if(
        &self,
        username: &str,
        id: u64,
    ) -> Option<ActorHandle<PeerMessage>> {
        let mut peers = match self.peers.lock_safe() {
            Ok(p) => p,
            Err(e) => {
                error!("[peer_registry] remove_peer_if: {}", e);
                return None;
            }
        };
        if peers
            .get(username)
            .is_some_and(|(stored, _, _)| *stored == id)
        {
            let removed = peers.remove(username).map(|(_, handle, _)| handle);
            debug!(
                "[peer_registry] Removed peer actor {} for {}",
                id, username
            );
            return removed;
        }
        None
    }

    #[must_use]
    pub fn contains(&self, username: &str) -> bool {
        match self.peers.lock_safe() {
            Ok(peers) => peers.contains_key(username),
            Err(e) => {
                error!("[peer_registry] contains: {}", e);
                false
            }
        }
    }

    pub fn send_to_peer(
        &self,
        username: &str,
        message: PeerMessage,
    ) -> Result<(), String> {
        let handle = self
            .get_peer(username)
            .ok_or_else(|| format!("Peer {username} not found in registry"))?;

        handle.send(message)
    }

    pub fn queue_upload(
        &self,
        username: &str,
        filename: String,
    ) -> Result<(), String> {
        self.send_to_peer(username, PeerMessage::QueueUpload(filename))
    }
}

impl Clone for PeerRegistry {
    fn clone(&self) -> Self {
        Self {
            peers: self.peers.clone(),
            actor_system: self.actor_system.clone(),
            client_channel: self.client_channel.clone(),
            own_username: self.own_username.clone(),
            capacity: self.capacity.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PeerRegistry;
    use crate::actor::ActorSystem;
    use crate::peer::{ConnectionType, Peer};
    use std::collections::HashSet;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn loopback_peer(name: &str) -> (Peer, TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_nonblocking(true).unwrap();
        let far_end = listener.accept().unwrap().0;
        let peer = Peer::new(
            name.to_string(),
            ConnectionType::P,
            "127.0.0.1".to_string(),
            u32::from(addr.port()),
            None,
            0,
            0,
            0,
        );
        (peer, stream, far_end)
    }

    fn capped_registry(
        capacity: usize,
    ) -> (
        PeerRegistry,
        std::sync::mpsc::Receiver<crate::client::ClientOperation>,
    ) {
        let system = Arc::new(ActorSystem::new());
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = PeerRegistry::with_capacity(
            system,
            tx,
            "me".to_string(),
            Arc::new(AtomicUsize::new(capacity)),
        );
        (registry, rx)
    }

    #[test]
    fn capacity_evicts_the_oldest_idle_peer() {
        let (registry, _rx) = capped_registry(2);

        for name in ["a", "b", "c"] {
            let (peer, stream, _far_end) = loopback_peer(name);
            registry.register_peer(peer, Some(stream), None).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(!registry.contains("a"), "the oldest peer must be evicted");
        assert!(registry.contains("b"));
        assert!(registry.contains("c"));
    }

    #[test]
    fn capacity_refuses_when_every_peer_is_protected() {
        let (registry, _rx) = capped_registry(1);

        let (busy, busy_stream, _busy_far) = loopback_peer("busy");
        registry
            .register_peer(busy, Some(busy_stream), None)
            .unwrap();

        let protected: HashSet<String> =
            std::iter::once("busy".to_string()).collect();
        let (newcomer, stream, _far) = loopback_peer("newcomer");
        let result = registry.register_peer_protected(
            newcomer,
            Some(stream),
            None,
            &protected,
        );

        assert!(result.is_err(), "a full registry of busy peers must refuse");
        assert!(registry.contains("busy"));
        assert!(!registry.contains("newcomer"));
    }

    #[test]
    fn a_returning_username_is_replaced_not_refused_at_capacity() {
        let (registry, _rx) = capped_registry(1);

        let (first, first_stream, _first_far) = loopback_peer("bob");
        registry
            .register_peer(first, Some(first_stream), None)
            .unwrap();

        let protected: HashSet<String> =
            std::iter::once("bob".to_string()).collect();
        let (again, stream, _far) = loopback_peer("bob");
        let result = registry.register_peer_protected(
            again,
            Some(stream),
            None,
            &protected,
        );

        assert!(
            result.is_ok(),
            "a reconnecting username must replace itself"
        );
        assert!(registry.contains("bob"));
    }

    #[test]
    fn remove_peer_if_respects_actor_identity() {
        let system = Arc::new(ActorSystem::new());
        let (tx, _rx) = std::sync::mpsc::channel();
        let registry = PeerRegistry::new(system, tx, "me".to_string());

        // A real loopback connection makes the actor inbound (no dial-out);
        // non-blocking so it can process Stop promptly on teardown.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_nonblocking(true).unwrap();
        let _server_side = listener.accept().unwrap().0;

        let peer = Peer::new(
            "bob".to_string(),
            ConnectionType::P,
            "127.0.0.1".to_string(),
            u32::from(addr.port()),
            None,
            0,
            0,
            0,
        );
        registry.register_peer(peer, Some(stream), None).unwrap();
        assert!(registry.contains("bob"));

        // A stale / wrong id must not evict the live actor.
        assert!(registry.remove_peer_if("bob", u64::MAX).is_none());
        assert!(registry.contains("bob"));

        // Unconditional removal still works (and stops the actor).
        let handle = registry.remove_peer("bob");
        assert!(handle.is_some());
        let _ = handle.unwrap().stop();
        assert!(!registry.contains("bob"));
    }

    // A dial that is refused reports its terminal outcome almost instantly —
    // possibly while register_peer is still between spawn and insert. The
    // registry holds its lock across both, so the eviction that follows must
    // always find the entry; a miss left a permanent zombie claiming the
    // username.
    #[test]
    fn refused_dial_does_not_leave_a_zombie_entry() {
        use crate::client::ClientOperation;

        let system = Arc::new(ActorSystem::new());
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = PeerRegistry::new(system, tx, "me".to_string());

        // A port with nothing listening behind it: bind, learn it, drop it.
        let port = {
            let probe = TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };

        let peer = Peer::new(
            "ghost".to_string(),
            ConnectionType::P,
            "127.0.0.1".to_string(),
            u32::from(port),
            None,
            0,
            0,
            0,
        );
        registry.register_peer(peer, None, None).unwrap();

        // Play the client ops loop: take the terminal outcome, evict by id.
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(ClientOperation::PeerConnectFailed(id, username)) => {
                assert_eq!(username, "ghost");
                if let Some(handle) = registry.remove_peer_if(&username, id) {
                    let _ = handle.stop();
                }
            }
            other => panic!("expected PeerConnectFailed, got {other:?}"),
        }
        assert!(
            !registry.contains("ghost"),
            "a refused dial must not leave a registry entry"
        );
    }
}
