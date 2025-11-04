use std::net::TcpListener;
use std::sync::mpsc::Sender;
use std::thread;

use crate::client::ClientOperation;

use crate::peer::{DefaultPeer, Peer};
use crate::{info, trace};

pub struct Listen {}

impl Listen {
    pub fn start(port: u32, client_sender: Sender<ClientOperation>) {
        let mut index = 0;

        info!("starting listener on port {port}");
        let listener = TcpListener::bind(format!("0.0.0.0:{port}")).unwrap();
        for stream in listener.incoming() {
            index += 1;
            let client_sender_clone = client_sender.clone();
            thread::spawn(move || {
                let stream = stream.unwrap();

                let peer_ip = stream.peer_addr().unwrap().ip().to_string();

                let peer = Peer::new(
                    format!("unknown-{index}"),
                    crate::peer::ConnectionType::P,
                    peer_ip.clone(),
                    port,
                    None,
                    0,
                    0,
                    0,
                );
                let default_peer = DefaultPeer::new(peer, client_sender_clone);

                trace!("[listener{index}] new connection from {peer_ip}");
                default_peer.connect_with_socket(stream).unwrap();
            });
        }
    }
}
