use crate::dispatcher::MessageDispatcher;
use crate::message::peer::FileSearchResponse;
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader};
use crate::types::FileSearchResult;

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::client::ClientOperation;
use crate::peer::Peer;
use std::io::{self, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[allow(dead_code)]
pub struct DefaultPeer {
    peer: Peer,
    peer_channel: Option<Sender<PeerOperation>>,
    client_channel: Sender<ClientOperation>,
    // Store thread handles for proper lifecycle management
    read_thread: Option<JoinHandle<()>>,
    write_thread: Option<JoinHandle<()>>,
}

#[allow(dead_code)]
pub enum PeerOperation {
    SendMessage(Message),
    FileSearchResult(FileSearchResult),
}
impl DefaultPeer {
    pub fn new(peer: Peer, client_channel: Sender<ClientOperation>) -> Self {
        Self {
            peer,
            peer_channel: None,
            client_channel,
            read_thread: None,
            write_thread: None,
        }
    }
    pub fn connect(mut self) -> Result<Self, io::Error> {
        let socket_address = format!("{}:{}", self.peer.host, self.peer.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid address"))?;

        let mut stream = TcpStream::connect_timeout(&socket_address, Duration::from_secs(10))?;

        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        if let Some(token) = self.peer.token.clone() {
            stream
                .write_all(&MessageFactory::build_watch_user(token.as_str()).get_data())
                .unwrap();
        }
        self.start_read_write_loops(stream)?;

        Ok(self)
    }

    fn start_read_write_loops(&mut self, stream: TcpStream) -> Result<(), io::Error> {
        let (peer_sender, peer_reader): (Sender<PeerOperation>, Receiver<PeerOperation>) =
            mpsc::channel();

        let mut read_stream = stream.try_clone()?;
        let mut write_stream = stream; // Use the original stream for writing

        let peer = self.peer.clone();
        let client_channel_for_read = self.client_channel.clone();

        // Spawn the reader thread
        self.read_thread = Some(thread::spawn(move || {
            let mut handlers = Handlers::new();
            handlers.register_handler(FileSearchResponse);

            let dispatcher = MessageDispatcher::new(peer_sender, handlers);

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        println!(
                            "Read operation timed out in default peer {:}:{:}",
                            peer.host, peer.port
                        );
                        continue;
                    }
                    Err(e) => {
                        eprintln!("Error reading from peer: {}. Terminating read loop.", e);
                        let _ = client_channel_for_read.send(ClientOperation::PeerDisconnected(peer.username.clone()));
                        break;
                    }
                }

                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => dispatcher.dispatch(&mut message),
                    Err(e) => {
                        println!(
                            "Error extracting message in default peer: {}. Terminating read loop.",
                            e
                        );
                        let _ = client_channel_for_read.send(ClientOperation::PeerDisconnected(peer.username.clone()));
                        break;
                    }
                    Ok(None) => continue,
                }
            }
        }));

        let client_channel = self.client_channel.clone();
        let peer_username = self.peer.username.clone();
        self.write_thread = Some(thread::spawn(move || {
            loop {
                match peer_reader.recv() {
                    Ok(operation) => {
                        match operation {
                            PeerOperation::SendMessage(message) => {
                                if let Err(e) = write_stream.write_all(&message.get_buffer()) {
                                    eprintln!("Error writing message to stream: {}. Terminating write loop.", e);
                                    let _ = client_channel.send(ClientOperation::PeerDisconnected(peer_username.clone()));
                                    break;
                                }
                            }
                            PeerOperation::FileSearchResult(file_search) => {
                                client_channel
                                    .send(ClientOperation::SearchResult(file_search))
                                    .unwrap();
                            }
                        }
                    }
                    Err(_) => {
                        // The sender has been dropped, the peer is shutting down.
                        println!("Peer channel closed. Terminating write loop.");
                        break;
                    }
                }
            }
        }));

        Ok(())
    }
}

impl Drop for DefaultPeer {
    fn drop(&mut self) {
        // Signal threads to shut down (e.g., by dropping the sender half of the channel)
        // For now, we'll just drop the peer_channel, which will cause the writer thread
        // to exit its loop. A more robust solution might involve a shared AtomicBool
        // or a dedicated shutdown channel.
        self.peer_channel = None;

        // Join the read thread
        if let Some(handle) = self.read_thread.take() {
            println!("Joining read thread...");
            match handle.join() {
                Ok(_) => println!("Read thread joined successfully."),
                Err(e) => eprintln!("Read thread panicked: {:?}", e),
            }
        }

        // Join the write thread
        if let Some(handle) = self.write_thread.take() {
            println!("Joining write thread...");
            match handle.join() {
                Ok(_) => println!("Write thread joined successfully."),
                Err(e) => eprintln!("Write thread panicked: {:?}", e),
            }
        }
    }
}
