use crate::dispatcher::MessageDispatcher;
use crate::message::peer::FileSearchResponse;
use crate::message::peer::{
    FolderContentsRequestHandler, GetShareFileList, PlaceInQueueResponse,
    QueueFailedHandler, TransferRequest, TransferResponse, UploadFailedHandler,
};
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader, MessageType};
use crate::types::{Download, FileSearchResult, Transfer};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::client::ClientOperation;
use crate::peer::Peer;
use crate::{debug, error, info, trace, warn};
use std::io::{self, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[allow(dead_code)]
pub struct DefaultPeer {
    peer: Peer,
    peer_channel: Option<Sender<PeerOperation>>,
    client_channel: Sender<ClientOperation>,
    pub read_thread: Option<JoinHandle<()>>,
    write_thread: Option<JoinHandle<()>>,
    has_active_download: Arc<Mutex<bool>>,
}

#[allow(dead_code)]
pub enum PeerOperation {
    SendMessage(Message),
    FileSearchResult(FileSearchResult),
    TransferRequest(Transfer),
    TransferResponse {
        token: Vec<u8>,
        allowed: bool,
        reason: Option<String>,
    },
    PlaceInQueueResponse {
        filename: String,
        place: u32,
    },
    UploadFailed {
        filename: String,
    },
}
impl DefaultPeer {
    pub fn new(peer: Peer, client_channel: Sender<ClientOperation>) -> Self {
        Self {
            peer,
            peer_channel: None,
            client_channel,
            read_thread: None,
            write_thread: None,
            has_active_download: Arc::new(Mutex::new(false)),
        }
    }
    pub fn connect(mut self) -> Result<Self, io::Error> {
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
        stream.set_nodelay(true)?;

        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        if let Some(token) = self.peer.token.clone() {
            stream
                .write_all(
                    &MessageFactory::build_watch_user(token).get_buffer(),
                )
                .unwrap();
        }
        self.start_read_write_loops(stream)?;

        Ok(self)
    }

    pub fn start_read_write_loops(
        &mut self,
        stream: TcpStream,
    ) -> Result<(), io::Error> {
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
        let has_active_download_read = self.has_active_download.clone();

        // Spawn the reader thread
        self.read_thread = Some(thread::spawn(move || {
            let mut handlers = Handlers::new();
            handlers.register_handler(FileSearchResponse);
            handlers.register_handler(TransferRequest);
            handlers.register_handler(TransferResponse);
            handlers.register_handler(GetShareFileList);
            handlers.register_handler(UploadFailedHandler);
            handlers.register_handler(PlaceInQueueResponse);
            handlers.register_handler(FolderContentsRequestHandler);
            handlers.register_handler(QueueFailedHandler);

            let dispatcher = MessageDispatcher::new(peer_sender, handlers);

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        debug!(
                            "Read operation timed out in default peer {:}:{:}",
                            peer.host, peer.port
                        );
                        continue;
                    }
                    Err(e) => {
                        error!("[default_peer:{}] Error reading from peer:  {}. Terminating read loop.", peer.username, e);
                        let _ = client_channel_for_read.send(
                            ClientOperation::PeerDisconnected(
                                peer.username.clone(),
                            ),
                        );
                        break;
                    }
                }

                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => {
                        // Log raw bytes for debugging - only for download peers
                        let message_code = message.get_message_code_u32();
                        if *has_active_download_read.lock().unwrap() {
                            debug!(
                                "[default_peer:{}] INCOMING RAW (code {}): {:?}",
                                peer.username,
                                message_code,
                                message.get_data()
                            );
                        }

                        trace!(
                            "[default_peer:{:?}] ← {:?}",
                            peer.username,
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
                        let _ = client_channel_for_read.send(
                            ClientOperation::PeerDisconnected(
                                peer.username.clone(),
                            ),
                        );
                        break;
                    }
                    Ok(None) => continue,
                }
            }
        }));

        let client_channel = self.client_channel.clone();
        let peer_channel = self.peer_channel.clone();
        let peer_username = self.peer.username.clone();
        let has_active_download_write = self.has_active_download.clone();

        self.write_thread = Some(thread::spawn(move || loop {
            match peer_reader.recv() {
                Ok(operation) => match operation {
                    PeerOperation::SendMessage(message) => {
                        let buff = message.get_buffer();

                        if *has_active_download_write.lock().unwrap() {
                            debug!(
                                "[default_peer:{}] OUTGOING RAW: {:?}",
                                peer_username, buff
                            );
                        }

                        if let Err(e) = write_stream.write_all(&buff) {
                            error!("Error writing message to stream: {} - {}. Terminating write loop.", peer_username, e);
                            let _ = client_channel.send(
                                ClientOperation::PeerDisconnected(
                                    peer_username.clone(),
                                ),
                            );
                            break;
                        }
                    }
                    PeerOperation::FileSearchResult(file_search) => {
                        client_channel
                            .send(ClientOperation::SearchResult(file_search))
                            .unwrap();
                    }
                    PeerOperation::TransferRequest(transfer) => {
                        debug!(
                            "[default_peer:{:}] TransferRequest for {:?}",
                            peer_username, transfer.token
                        );

                        client_channel
                            .send(ClientOperation::ChangeDownload(
                                transfer.clone(),
                                peer_username.clone(),
                            ))
                            .unwrap();

                        let transfer_response =
                            MessageFactory::build_transfer_response_message(
                                transfer.clone(),
                            );

                        thread::sleep(Duration::from_millis(200));

                        if let Some(sender) = peer_channel.clone() {
                            sender
                                .send(PeerOperation::SendMessage(
                                    transfer_response.clone(),
                                ))
                                .unwrap();
                            debug!(
                            "[default_peer:{:}] Sent TransferResponse for token {:?}, {:?}",
                            peer_username, transfer.token, &transfer_response.get_buffer()
                        );
                        }
                    }
                    PeerOperation::TransferResponse {
                        token,
                        allowed,
                        reason,
                    } => {
                        debug!(
                                    "[default_peer:{}] transfer response token: {:?} allowed: {}",
                                    peer_username, token, allowed
                                );

                        if !allowed {
                            // client_channel
                            //     .send(ClientOperation::RemoveDownload(
                            //         token.clone(),
                            //     ))
                            //     .unwrap();
                            if let Some(reason_text) = reason {
                                debug!(
                                        "[default_peer:{}] Transfer rejected: {} - token {:?}, I will receive TransferRequest soon...",
                                        peer_username.clone(),
                                        reason_text,
                                        token.clone()
                                    );
                            }
                        } else {
                            debug!("[default_peer:{:}] Transfer allowed, sending GetPeerAddress for token {:?}",peer_username, token);
                            // Send GetPeerAddress to server to get ConnectToPeer type F
                            client_channel
                                .send(ClientOperation::GetPeerAddress(
                                    peer_username.clone(),
                                    token,
                                ))
                                .unwrap();
                        }
                    }
                    PeerOperation::PlaceInQueueResponse { filename, place } => {
                        debug!(
                            "[default_peer:{}] Place in queue response - file: {}, place: {}",
                            peer_username, filename, place
                        );
                    }
                    PeerOperation::UploadFailed { filename } => {
                        info!(
                            "[default_peer:{}] Upload failed for {}",
                            peer_username, filename
                        );
                    }
                },
                Err(_) => {
                    debug!("[default_peer:{}] Peer channel closed. Terminating write loop.", peer_username);
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
        // Set the flag to indicate this peer has an active download
        *self.has_active_download.lock().unwrap() = true;

        let message = MessageFactory::build_transfer_request_message(
            &download.filename,
            download.token,
        );
        if let Some(sender) = &self.peer_channel {
            sender.send(PeerOperation::SendMessage(message)).unwrap();
        }
        Ok(())
    }

    pub fn file_search_result(
        &self,
        files: Vec<crate::share::SharedFile>,
        ticket: Vec<u8>,
        own_username: String,
    ) {
        debug!(
            "[default_peer:{}] Sending {} search results with ticket {:?}",
            self.peer.username,
            files.len(),
            ticket
        );

        let message = MessageFactory::build_file_search_result_message(
            files,
            ticket,
            own_username,
        );

        if let Some(sender) = &self.peer_channel {
            sender.send(PeerOperation::SendMessage(message)).unwrap();
        }
    }
}

impl Drop for DefaultPeer {
    fn drop(&mut self) {
        self.peer_channel = None;

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
