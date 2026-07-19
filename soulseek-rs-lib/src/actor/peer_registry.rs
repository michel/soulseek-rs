use crate::actor::peer_actor::{PeerActor, PeerMessage};
use crate::actor::{ActorHandle, ActorSystem};
use crate::client::ClientOperation;
use crate::message::MessageReader;
use crate::peer::Peer;
use crate::utils::lock::MutexExt;
use crate::{debug, error};

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Source of unique per-actor ids so terminal-outcome eviction can be made
/// identity-aware (a replaced actor must not evict its replacement).
static NEXT_PEER_ID: AtomicU64 = AtomicU64::new(1);

/// Registered peers keyed by username, each stored with the unique id of the
/// actor currently occupying the slot.
type PeerMap = HashMap<String, (u64, ActorHandle<PeerMessage>)>;

pub struct PeerRegistry {
    peers: Arc<Mutex<PeerMap>>,
    actor_system: Arc<ActorSystem>,
    client_channel: ActorHandle<ClientOperation>,
    own_username: String,
}

impl PeerRegistry {
    #[must_use]
    pub fn new(
        actor_system: Arc<ActorSystem>,
        client_channel: ActorHandle<ClientOperation>,
        own_username: String,
    ) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            actor_system,
            client_channel,
            own_username,
        }
    }

    pub fn register_peer(
        &self,
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
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

        let handle =
            self.actor_system.spawn_with_handle(actor, |actor, handle| {
                actor.set_self_handle(handle);
            });

        let mut peers = self
            .peers
            .lock_safe()
            .map_err(|e| format!("peer registry lock poisoned: {e}"))?;
        // Stop any actor already registered under this username so it does not
        // become an orphan pinning a pool worker forever (and so its later
        // shutdown cannot remove this new connection from the registry).
        if let Some((_, old_handle)) =
            peers.insert(username.clone(), (id, handle.clone()))
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
            Ok(peers) => peers.get(username).map(|(_, handle)| handle.clone()),
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

        removed.map(|(_, handle)| handle)
    }

    /// Remove and return the actor for `username` only if it is still the actor
    /// with `id`. A stale actor's terminal notification therefore cannot evict
    /// the newer actor that now occupies the slot.
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
        if peers.get(username).is_some_and(|(stored, _)| *stored == id) {
            let removed = peers.remove(username).map(|(_, handle)| handle);
            debug!(
                "[peer_registry] Removed peer actor {} for {}",
                id, username
            );
            return removed;
        }
        None
    }

    #[must_use]
    pub fn get_all_usernames(&self) -> Vec<String> {
        match self.peers.lock_safe() {
            Ok(peers) => peers.keys().cloned().collect(),
            Err(e) => {
                error!("[peer_registry] get_all_usernames: {}", e);
                Vec::new()
            }
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        match self.peers.lock_safe() {
            Ok(peers) => peers.len(),
            Err(e) => {
                error!("[peer_registry] count: {}", e);
                0
            }
        }
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
        }
    }
}
