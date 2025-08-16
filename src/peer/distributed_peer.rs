use crate::client::ClientOperation;
use crate::debug;
use crate::dispatcher::MessageDispatcher;
use crate::message::peer::distributed::{
    BranchLevel, BranchRoot, SearchRequest,
};
use crate::message::{server::MessageFactory, Handlers, MessageReader};
use crate::peer::{ConnectionType, Peer};
use std::io::{self, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[allow(dead_code)]
pub struct DistributedPeer {
    peer: Peer,
    peer_channel: Option<Sender<()>>,
    client_channel: Sender<ClientOperation>,
    read_thread: Option<JoinHandle<()>>,
    write_thread: Option<JoinHandle<()>>,
}

impl DistributedPeer {
    pub fn new(peer: Peer, client_channel: Sender<ClientOperation>) -> Self {
        Self {
            peer,
            peer_channel: None,
            client_channel,
            read_thread: None,
            write_thread: None,
        }
    }

    pub fn connect(mut self, _own_username: &str) -> Result<Self, io::Error> {
        let socket_address = format!("{}:{}", self.peer.host, self.peer.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Invalid address")
            })?;

        let mut stream = TcpStream::connect_timeout(
            &socket_address,
            Duration::from_secs(20),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(300)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        if let Some(token) = self.peer.token.clone() {
            let peer_init = MessageFactory::build_peer_init_message(
                &self.peer.username, // Use the peer's username
                ConnectionType::D,
                token.clone(),
            );
            stream.write_all(&peer_init.get_buffer())?;

            let pierce_fw = MessageFactory::build_watch_user(token);
            stream.write_all(&pierce_fw.get_data())?;
        }

        self.start_read_write_loops(stream)?;
        Ok(self)
    }

    fn start_read_write_loops(
        &mut self,
        stream: TcpStream,
    ) -> Result<(), io::Error> {
        let (_peer_sender, peer_reader): (Sender<()>, Receiver<()>) =
            mpsc::channel();
        self.peer_channel = Some(_peer_sender.clone());

        let mut read_stream = stream.try_clone()?;
        let _write_stream = stream;

        let peer = self.peer.clone();
        let client_channel_for_read = self.client_channel.clone();

        self.read_thread = Some(thread::spawn(move || {
            let mut handlers: Handlers<ClientOperation> = Handlers::new();
            handlers.register_handler(SearchRequest);
            handlers.register_handler(BranchLevel);
            handlers.register_handler(BranchRoot);

            let dispatcher = MessageDispatcher::new(
                client_channel_for_read.clone(),
                handlers,
            );
            let mut buffered_reader = MessageReader::new();

            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        continue
                    }
                    Err(e) => {
                        debug!(
                            "[dist_peer:{}] Disconnected: {}. Closing.",
                            peer.username, e
                        );
                        let _ = client_channel_for_read.send(
                            ClientOperation::PeerDisconnected(
                                peer.username.clone(),
                            ),
                        );
                        break;
                    }
                }
                while let Ok(Some(mut message)) =
                    buffered_reader.extract_message()
                {
                    dispatcher.dispatch(&mut message);
                }
            }
        }));

        self.write_thread = Some(thread::spawn(move || loop {
            if peer_reader.recv().is_err() {
                break;
            }
        }));

        Ok(())
    }
}

impl Drop for DistributedPeer {
    fn drop(&mut self) {
        debug!("Dropping DistributedPeer for {}", self.peer.username);

        if let Some(sender) = &self.peer_channel {
            let _ = sender.send(());
        }

        if let Some(thread) = self.read_thread.take() {
            let _ = thread.join();
        }

        if let Some(thread) = self.write_thread.take() {
            let _ = thread.join();
        }
    }
}

