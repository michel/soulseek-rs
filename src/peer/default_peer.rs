use crate::dispatcher::MessageDispatcher;
use crate::message::peer::{FileSearchResponse, PeerInit};
use crate::message::peer::{
    GetShareFileList, PlaceInQueueResponse, TransferRequest, TransferResponse,
    UploadFailedHandler,
};
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader, MessageType};
use crate::types::{Download, FileSearchResult, Transfer};

use core::result::Result::Ok;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, RwLock};
use std::thread::{self, JoinHandle};

use crate::client::ClientOperation;
use crate::peer::Peer;
use crate::{debug, error, trace, warn};
use std::io::{self, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug)]
pub struct DefaultPeer {
    peer: Arc<RwLock<Peer>>,
    peer_channel: Option<Sender<PeerOperation>>,
    client_channel: Sender<ClientOperation>,
    read_thread: Option<JoinHandle<()>>,
    write_thread: Option<JoinHandle<()>>,
}

#[allow(dead_code)]
pub enum PeerOperation {
    SendMessage(Message),
    FileSearchResult(FileSearchResult),
    TransferRequest(Transfer),
    TransferResponse {
        token: u32,
        allowed: bool,
        reason: Option<String>,
    },
    PlaceInQueueResponse {
        filename: String,
        place: u32,
    },
    SetUsername(String),
}

impl DefaultPeer {
    pub fn new(peer: Peer, client_channel: Sender<ClientOperation>) -> Self {
        Self {
            peer: Arc::new(RwLock::new(peer)),
            peer_channel: None,
            client_channel,
            read_thread: None,
            write_thread: None,
        }
    }
    pub fn disconnect(mut self) {
        let username = self.peer.read().unwrap().username.clone();
        debug!("[default_peer:{}] disconnect", username);
        if let Err(e) = self
            .client_channel
            .send(ClientOperation::PeerDisconnected(username))
        {
            error!("Failed to send disconnect notification: {}", e);
        }

        self.peer_channel.take();
    }

    pub fn connect_with_socket(
        mut self,
        mut stream: TcpStream,
    ) -> Result<Self, io::Error> {
        if let Some(token) = self.peer.read().unwrap().token {
            let mut message: Vec<u8> = [0, 5, 0, 0, 0, 0].to_vec();
            message.extend_from_slice(&token.to_le_bytes());
            stream.write_all(&message).unwrap();
        }

        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;

        trace!(
            "[default_peer:{}] connect_with_socket: direct",
            self.peer.read().unwrap().username
        );
        self.start_read_write_loops(stream)?;

        Ok(self)
    }

    pub fn connect(mut self) -> Result<Self, io::Error> {
        {
            let peer = self.peer.read().unwrap();
            println!(
                "[default_peer] Connecting to {} on port {}",
                peer.host, peer.port
            );
        }
        let socket_address = {
            let peer = self.peer.read().unwrap();
            format!("{}:{}", peer.host, peer.port)
        }
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid address")
        })?;

        trace!(
            "[default_peer:{}] connect: indirect",
            self.peer.read().unwrap().username
        );
        let mut stream = TcpStream::connect_timeout(
            &socket_address,
            Duration::from_secs(20),
        )?;

        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;

        if let Some(token) = self.peer.read().unwrap().token {
            stream
                .write_all(&MessageFactory::build_watch_user(token).get_data())
                .unwrap();
        }
        self.start_read_write_loops(stream)?;

        Ok(self)
    }

    fn start_read_write_loops(
        &mut self,
        stream: TcpStream,
    ) -> Result<(), io::Error> {
        debug!(
            "[default_peer:{}] start_read_write_loops",
            self.peer.read().unwrap().username
        );
        let (peer_sender, peer_reader): (
            Sender<PeerOperation>,
            Receiver<PeerOperation>,
        ) = mpsc::channel();

        // Set the peer_channel so transfer_request can send messages
        self.peer_channel = Some(peer_sender.clone());

        let mut read_stream = stream.try_clone()?;
        let mut write_stream = stream; // Use the original stream for writing

        let peer = self.peer.clone();
        let peer_clone = self.peer.clone();
        let client_channel_for_read = self.client_channel.clone();

        self.read_thread = Some(thread::spawn(move || {
            let mut handlers = Handlers::new();
            handlers.register_handler(FileSearchResponse);
            handlers.register_handler(TransferRequest);
            handlers.register_handler(TransferResponse);
            handlers.register_handler(GetShareFileList);
            handlers.register_handler(UploadFailedHandler);
            handlers.register_handler(PlaceInQueueResponse);
            handlers.register_handler(PeerInit);

            let dispatcher = MessageDispatcher::new(
                "default_peer".to_string(),
                peer_sender,
                handlers,
            );

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(()) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        {
                            let peer_lock = peer.read().unwrap();
                            debug!(
                                "Read operation timed out in default peer {:}:{:}",
                                peer_lock.host, peer_lock.port
                            );
                        }
                        continue;
                    }
                    Err(e) => {
                        let username = peer.read().unwrap().username.clone();
                        error!("[default_peer:{}] Error reading from peer:  {}. Terminating read loop.",username, e);
                        let _ = client_channel_for_read
                            .send(ClientOperation::PeerDisconnected(username));
                        break;
                    }
                }

                // Extract all available messages from buffer
                let mut should_terminate = false;
                loop {
                    match buffered_reader.extract_message() {
                        Ok(Some(mut message)) => {
                            trace!(
                                "[default_peer:{}] ← {:?}",
                                peer.read().unwrap().username,
                                message
                                    .get_message_name(
                                        MessageType::Peer,
                                        message.get_message_code() as u32
                                    )
                                    .map_err(|e| e.to_string())
                            );
                            dispatcher.dispatch(&mut message)
                        }
                        Err(e) => {
                            warn!(
                                "Error extracting message in default peer: {}. Terminating read loop.",
                                e
                            );
                            let username =
                                peer.read().unwrap().username.clone();
                            let _ = client_channel_for_read.send(
                                ClientOperation::PeerDisconnected(username),
                            );
                            should_terminate = true;
                            break;
                        }
                        Ok(_) => break,
                    }
                }
                if should_terminate {
                    break;
                }
            }
        }));

        let client_channel = self.client_channel.clone();
        let peer_channel = self.peer_channel.clone();
        let peer_clone_for_write = self.peer.clone();

        self.write_thread = Some(thread::spawn(move || loop {
            match peer_reader.recv() {
                Ok(operation) => match operation {
                    PeerOperation::SendMessage(message) => {
                        trace!(
                            "[default_peer:{}] ➡ {:?} - {:?}",
                            peer_clone_for_write.read().unwrap().username,
                            message
                                .get_message_name(
                                    MessageType::Peer,
                                    u32::from_le_bytes(
                                        message
                                            .get_slice(0, 4)
                                            .try_into()
                                            .unwrap()
                                    )
                                )
                                .map_err(|e| e.to_string()),
                            u32::from_le_bytes(
                                message.get_slice(0, 4).try_into().unwrap()
                            )
                        );

                        if let Err(e) =
                            write_stream.write_all(&message.get_buffer())
                        {
                            error!("Error writing message to stream: {} - {}. Terminating write loop.", peer_clone_for_write.read().unwrap().username, e);
                            let _ = client_channel.send(
                                ClientOperation::PeerDisconnected(
                                    peer_clone_for_write
                                        .read()
                                        .unwrap()
                                        .username
                                        .clone(),
                                ),
                            );
                            break;
                        }
                        write_stream.flush().unwrap();
                    }
                    PeerOperation::FileSearchResult(file_search) => {
                        client_channel
                            .send(ClientOperation::SearchResult(file_search))
                            .unwrap();
                    }
                    PeerOperation::TransferRequest(transfer) => {
                        debug!(
                            "[default_peer:{}] TransferRequest for {}",
                            peer_clone_for_write.read().unwrap().username,
                            transfer.token
                        );
                        client_channel
                            .send(ClientOperation::UpdateDownloadTokens(
                                transfer.clone(),
                                peer_clone.read().unwrap().username.clone(),
                            ))
                            .unwrap();

                        let transfer_response =
                            MessageFactory::build_transfer_response_message(
                                transfer.clone(),
                            );

                        trace!(
                            "[default_peer:{}] TransferResponse for {:?}",
                            peer_clone_for_write.read().unwrap().username,
                            transfer_response.get_buffer()
                        );
                        if let Some(sender) = peer_channel.clone() {
                            sender
                                .send(PeerOperation::SendMessage(
                                    transfer_response,
                                ))
                                .unwrap();
                        }
                    }
                    PeerOperation::TransferResponse {
                        token,
                        allowed,
                        reason,
                    } => {
                        debug!(
                                                    "[default_peer:{}] transfer response token: {} allowed: {}",
                                                    peer_clone_for_write.read().unwrap().username, token, allowed
                                                );

                        if !allowed {
                            if let Some(reason_text) = reason {
                                debug!(
                                                        "[default_peer:{}] Transfer rejected: {} - token {}, I will receive TransferRequest soon...",
                                                        peer_clone_for_write.read().unwrap().username.clone(),
                                                        reason_text,
                                                        token
                                                    );
                            }
                        } else {
                            debug!("[default_peer:{}] Transfer allowed, ready to connect with token {:}",peer_clone_for_write.read().unwrap().username, token);
                            client_channel
                                .send(ClientOperation::DownloadFromPeer(
                                    token,
                                    peer_clone.read().unwrap().clone(),
                                ))
                                .unwrap();
                        }
                    }
                    PeerOperation::PlaceInQueueResponse { filename, place } => {
                        debug!(
                                            "[default_peer:{}] Place in queue response - file: {}, place: {}",
                                            peer_clone_for_write.read().unwrap().username, filename, place
                                        );
                    }
                    PeerOperation::SetUsername(username) => {
                        trace!(
                            "[default_peer:{}] SetUsername: {}",
                            peer_clone_for_write.read().unwrap().username,
                            username
                        );

                        peer_clone.write().unwrap().username = username;
                    }
                },
                Err(_) => {
                    debug!("[default_peer:{}] Peer channel closed. Terminating write loop.", peer_clone_for_write.read().unwrap().username);
                    break;
                }
            }
        }));

        Ok(())
    }

    pub fn transfer_request(
        &self,
        download: Download,
    ) -> Result<(), io::Error> {
        let message = MessageFactory::build_transfer_request_message(
            &download.filename,
            download.token,
        );
        if let Some(sender) = &self.peer_channel {
            sender.send(PeerOperation::SendMessage(message)).unwrap();
        }
        Ok(())
    }
}

impl Drop for DefaultPeer {
    fn drop(&mut self) {
        self.peer_channel = None;
        trace!("[default_peer:{}] drop", self.peer.read().unwrap().username);

        if let Some(handle) = self.read_thread.take() {
            debug!("Joining read thread...");
            match handle.join() {
                Ok(_) => debug!("Read thread joined successfully."),
                Err(e) => error!("Read thread panicked: {:?}", e),
            }
        }

        if let Some(handle) = self.write_thread.take() {
            debug!("Joining write thread...");
            match handle.join() {
                Ok(_) => debug!("Write thread joined successfully."),
                Err(e) => error!("Write thread panicked: {:?}", e),
            }
        }
    }
}
