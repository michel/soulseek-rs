use crate::dispatcher::MessageDispatcher;
use crate::message::peer::{FileSearch, FileSearchResponse};
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Barrier};
use std::thread::{self};

use crate::peer::Peer;
use crate::client::ClientOperation;
use std::io::{self, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[allow(dead_code)]
pub struct DefaultPeer {
    peer: Peer,
    // client_channel: Sender<ClientOperation>,
    peer_channel: Option<Sender<PeerOperation>>,
}

#[allow(dead_code)]
pub enum PeerOperation {
    SendMessage(Message),
    FileSearchResult(FileSearch),
}
impl DefaultPeer {
    pub fn new(peer: Peer, _client_channel: Sender<ClientOperation>) -> Self {
        Self {
            peer,
            // client_channel,
            peer_channel: None,
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
        self.start_read_write_loops(stream).unwrap();

        Ok(self)
    }

    fn start_read_write_loops(&mut self, stream: TcpStream) -> Result<(), io::Error> {
        let (peer_sender, peer_reader): (Sender<PeerOperation>, Receiver<PeerOperation>) =
            mpsc::channel();

        let sender = peer_sender.clone();
        let barrier = Arc::new(Barrier::new(3));
        let read_barrier = barrier.clone();
        let write_barrier = barrier.clone();
        let done_barrier = barrier.clone();

        let mut read_stream = stream.try_clone()?;
        let mut write_stream = stream.try_clone()?;

        let peer = self.peer.clone();
        thread::spawn(move || {
            read_barrier.wait();

            let mut handlers = Handlers::new();
            handlers.register_handler(FileSearchResponse);

            let dispatcher = MessageDispatcher::new(sender, handlers);

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        println!(
                            "Read operation timed out in default peer {:}:{:}",
                            peer.host, peer.port
                        );
                        continue;
                    }
                    Err(e) => {
                        eprintln!("Error reading from peer: {}", e);
                        break;
                    }
                }

                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => dispatcher.dispatch(&mut message),
                    Err(e) => {
                        println!("Error extracting message in default peer: {}", e)
                    }
                    Ok(None) => continue,
                }
            }
        });

        thread::spawn(move || {
            write_barrier.wait();
            loop {
                if let Ok(operation) = peer_reader.recv() {
                    match operation {
                        PeerOperation::SendMessage(message) => {
                            match write_stream.write_all(&message.get_buffer()) {
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("Error writing message to stream : {}", e);
                                    break;
                                }
                            }
                        }
                        PeerOperation::FileSearchResult(file_search) => {
                            println!("{:?}", file_search)
                        }
                    }
                }
            }
        });
        done_barrier.wait();
        Ok(())
    }
}
