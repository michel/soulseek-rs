use crate::actor::peer_actor::{PeerActor, PeerMessage};
use crate::actor::{ActorHandle, ActorSystem};
use crate::client::ClientOperation;
use crate::debug;
use crate::message::MessageReader;
use crate::peer::Peer;

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub struct PeerRegistry {
    peers: Arc<Mutex<HashMap<String, ActorHandle<PeerMessage>>>>,
    actor_system: Arc<ActorSystem>,
    client_channel: Sender<ClientOperation>,
}

impl PeerRegistry {
    pub fn new(
        actor_system: Arc<ActorSystem>,
        client_channel: Sender<ClientOperation>,
    ) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            actor_system,
            client_channel,
        }
    }

    pub fn register_peer(
        &self,
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
    ) -> Result<ActorHandle<PeerMessage>, String> {
        let username = peer.username.clone();

        let actor =
            PeerActor::new(peer, stream, reader, self.client_channel.clone());

        let handle =
            self.actor_system.spawn_with_handle(actor, |actor, handle| {
                actor.set_self_handle(handle);
            });

        let mut peers = self.peers.lock().unwrap();
        peers.insert(username.clone(), handle.clone());

        Ok(handle)
    }

    pub fn get_peer(&self, username: &str) -> Option<ActorHandle<PeerMessage>> {
        let peers = self.peers.lock().unwrap();
        peers.get(username).cloned()
    }

    pub fn remove_peer(
        &self,
        username: &str,
    ) -> Option<ActorHandle<PeerMessage>> {
        let mut peers = self.peers.lock().unwrap();
        let handle = peers.remove(username);

        if handle.is_some() {
            debug!("[peer_registry] Removed peer actor for {}", username);
        }

        handle
    }

    pub fn get_all_usernames(&self) -> Vec<String> {
        let peers = self.peers.lock().unwrap();
        peers.keys().cloned().collect()
    }

    pub fn count(&self) -> usize {
        let peers = self.peers.lock().unwrap();
        peers.len()
    }

    pub fn contains(&self, username: &str) -> bool {
        let peers = self.peers.lock().unwrap();
        peers.contains_key(username)
    }

    pub fn send_to_peer(
        &self,
        username: &str,
        message: PeerMessage,
    ) -> Result<(), String> {
        let handle = self.get_peer(username).ok_or_else(|| {
            format!("Peer {} not found in registry", username)
        })?;

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
        }
    }
}
