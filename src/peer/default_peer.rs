use crate::server::ServerAddress;

use super::peer::Peer;

pub struct DefaultPeer {}
impl DefaultPeer {
    pub fn new(address: ServerAddress, peer: Peer) -> Self {
        Self {}
    }
}
