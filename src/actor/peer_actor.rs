use crate::actor::{Actor, ActorHandle};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::peer::{
    FileSearchResponse, GetShareFileList, PeerInit, PlaceInQueueResponse,
    TransferRequest, TransferResponse, UploadFailedHandler,
};
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader, MessageType};
use crate::peer::Peer;
use crate::types::{Download, FileSearchResult, Transfer};
use crate::{debug, error, trace, warn};

use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};

/// Messages that can be sent to a PeerActor
#[derive(Debug, Clone)]
pub enum PeerMessage {
    /// Send a raw message to the peer
    SendMessage(Message),
    /// Handle a file search result
    FileSearchResult(FileSearchResult),
    /// Handle a transfer request
    TransferRequest(Transfer),
    /// Handle a transfer response
    TransferResponse {
        token: u32,
        allowed: bool,
        reason: Option<String>,
    },
    /// Handle a place in queue response
    PlaceInQueueResponse { filename: String, place: u32 },
    /// Set the peer's username
    SetUsername(String),
    /// Queue an upload for a file
    QueueUpload(String),
    /// Request a transfer for a download
    RequestTransfer(Download),
    /// Internal: process read operations
    ProcessRead,
}

/// Actor that manages a single peer connection
pub struct PeerActor {
    peer: Arc<RwLock<Peer>>,
    stream: Option<TcpStream>,
    reader: MessageReader,
    client_channel: Sender<ClientOperation>,
    self_handle: Option<ActorHandle<PeerMessage>>,
    dispatcher: Option<MessageDispatcher<PeerMessage>>,
    dispatcher_receiver: Option<Receiver<PeerMessage>>,
}

impl PeerActor {
    /// Create a new PeerActor with an existing socket connection
    pub fn new(
        peer: Peer,
        stream: TcpStream,
        reader: Option<MessageReader>,
        client_channel: Sender<ClientOperation>,
    ) -> Self {
        Self {
            peer: Arc::new(RwLock::new(peer)),
            stream: Some(stream),
            reader: reader.unwrap_or_default(),
            client_channel,
            self_handle: None,
            dispatcher: None,
            dispatcher_receiver: None,
        }
    }

    /// Set the actor's own handle for self-messaging
    pub fn set_self_handle(&mut self, handle: ActorHandle<PeerMessage>) {
        self.self_handle = Some(handle);
    }

    /// Initialize the message dispatcher with handlers
    fn initialize_dispatcher(&mut self) {
        // Create a channel for the dispatcher to send messages to
        let (dispatcher_sender, dispatcher_receiver) =
            std::sync::mpsc::channel::<PeerMessage>();

        self.dispatcher_receiver = Some(dispatcher_receiver);

        let mut handlers = Handlers::new();
        handlers.register_handler(FileSearchResponse);
        handlers.register_handler(TransferRequest);
        handlers.register_handler(TransferResponse);
        handlers.register_handler(GetShareFileList);
        handlers.register_handler(UploadFailedHandler);
        handlers.register_handler(PlaceInQueueResponse);
        handlers.register_handler(PeerInit);

        self.dispatcher = Some(MessageDispatcher::new(
            "peer_actor".to_string(),
            dispatcher_sender,
            handlers,
        ));
    }

    /// Process any messages from the dispatcher
    fn process_dispatcher_messages(&mut self) {
        // Collect all messages first to avoid borrow issues
        let messages: Vec<PeerMessage> =
            if let Some(ref receiver) = self.dispatcher_receiver {
                let mut msgs = Vec::new();
                while let Ok(msg) = receiver.try_recv() {
                    msgs.push(msg);
                }
                msgs
            } else {
                Vec::new()
            };

        // Process all collected messages
        for msg in messages {
            self.handle_message(msg);
        }
    }

    /// Internal handler that doesn't call Actor::handle
    fn handle_message(&mut self, msg: PeerMessage) {
        match msg {
            PeerMessage::SendMessage(message) => {
                self.send_message(message);
            }
            PeerMessage::FileSearchResult(file_search) => {
                self.client_channel
                    .send(ClientOperation::SearchResult(file_search))
                    .unwrap();
            }
            PeerMessage::TransferRequest(transfer) => {
                let username = self.peer.read().unwrap().username.clone();
                debug!(
                    "[peer_actor:{}] TransferRequest for {}",
                    username, transfer.token
                );

                self.client_channel
                    .send(ClientOperation::UpdateDownloadTokens(
                        transfer.clone(),
                        username.clone(),
                    ))
                    .unwrap();

                let transfer_response =
                    MessageFactory::build_transfer_response_message(transfer);

                if let Some(ref handle) = self.self_handle {
                    if let Err(e) =
                        handle.send(PeerMessage::SendMessage(transfer_response))
                    {
                        error!("[peer_actor:{}] Failed to send TransferResponse message: {}", username, e);
                    }
                }
            }
            PeerMessage::TransferResponse {
                token,
                allowed,
                reason,
            } => {
                let username = self.peer.read().unwrap().username.clone();
                debug!(
                    "[peer_actor:{}] transfer response token: {} allowed: {}",
                    username, token, allowed
                );

                if !allowed {
                    if let Some(reason_text) = reason {
                        debug!(
                            "[peer_actor:{}] Transfer rejected: {} - token {}, waiting for TransferRequest...",
                            username, reason_text, token
                        );
                    }
                } else {
                    debug!(
                        "[peer_actor:{}] Transfer allowed, ready to connect with token {:}",
                        username, token
                    );
                    self.client_channel
                        .send(ClientOperation::DownloadFromPeer(
                            token,
                            self.peer.read().unwrap().clone(),
                            allowed,
                        ))
                        .unwrap();
                }
            }
            PeerMessage::PlaceInQueueResponse { filename, place } => {
                let username = self.peer.read().unwrap().username.clone();
                debug!(
                    "[peer_actor:{}] Place in queue response - file: {}, place: {}",
                    username, filename, place
                );
            }
            PeerMessage::SetUsername(username) => {
                trace!(
                    "[peer_actor:{}] SetUsername: {}",
                    self.peer.read().unwrap().username,
                    username
                );
                self.peer.write().unwrap().username = username;
            }
            PeerMessage::QueueUpload(filename) => {
                let message =
                    MessageFactory::build_queue_upload_message(&filename);
                self.send_message(message);
            }
            PeerMessage::RequestTransfer(download) => {
                let message = MessageFactory::build_transfer_request_message(
                    &download.filename,
                    download.token,
                );
                self.send_message(message);
            }
            PeerMessage::ProcessRead => {
                self.process_read();
            }
        }
    }

    /// Handle reading from the socket and dispatching messages
    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages(); // This will process all messages in the buffer
        }
        let username = self.peer.read().unwrap().username.clone();

        // Try to read more data from the socket first
        {
            let stream = match self.stream.as_mut() {
                Some(s) => s,
                None => {
                    trace!("[peer_actor:{}] process_read: stream is None, returning", username);
                    return;
                }
            };

            match self.reader.read_from_socket(stream) {
                Ok(()) => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    let peer_lock = self.peer.read().unwrap();
                    debug!(
                        "Read operation timed out for peer actor {:}:{:}",
                        peer_lock.host, peer_lock.port
                    );
                }
                Err(e) => {
                    let username = self.peer.read().unwrap().username.clone();
                    error!(
                        "[peer_actor:{}] Error reading from peer: {} (kind: {:?}). Disconnecting.",
                        username, e, e.kind()
                    );
                    self.disconnect();
                    return;
                }
            }
        }
        self.extract_and_process_messages(); // This will process all messages in the buffer
    }

    fn extract_and_process_messages(&mut self) {
        let username = self.peer.read().unwrap().username.clone();
        let mut extracted_count = 0;
        loop {
            match self.reader.extract_message() {
                Ok(Some(mut message)) => {
                    extracted_count += 1;
                    trace!(
                        "[peer_actor:{}] ← Message #{}: {:?}",
                        username,
                        extracted_count,
                        message
                            .get_message_name(
                                MessageType::Peer,
                                message.get_message_code() as u32
                            )
                            .map_err(|e| e.to_string())
                    );
                    if let Some(ref dispatcher) = self.dispatcher {
                        dispatcher.dispatch(&mut message);
                    } else {
                        warn!(
                            "[peer_actor:{}] No dispatcher available!",
                            username
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "[peer_actor:{}] Error extracting message: {}. Disconnecting peer.",
                        username, e
                    );
                    self.disconnect();
                    return;
                }
                Ok(None) => {
                    break;
                }
            }
        }

        // Process any messages that were dispatched
        self.process_dispatcher_messages();
    }

    /// Handle writing a message to the socket
    fn send_message(&mut self, message: Message) {
        let stream = match self.stream.as_mut() {
            Some(s) => s,
            None => {
                error!("Cannot send message: stream is None");
                return;
            }
        };

        let username = self.peer.read().unwrap().username.clone();
        trace!(
            "[peer_actor:{}] ➡ {:?}",
            username,
            message
                .get_message_name(
                    MessageType::Peer,
                    u32::from_le_bytes(
                        message.get_slice(0, 4).try_into().unwrap()
                    )
                )
                .map_err(|e| e.to_string())
        );

        if let Err(e) = stream.write_all(&message.get_buffer()) {
            error!(
                "[peer_actor:{}] Error writing message: {}. Disconnecting.",
                username, e
            );
            self.disconnect();
            return;
        }

        if let Err(e) = stream.flush() {
            error!(
                "[peer_actor:{}] Error flushing stream: {}. Disconnecting.",
                username, e
            );
            self.disconnect();
        }
    }

    /// Disconnect the peer and notify the client
    fn disconnect(&mut self) {
        let username = self.peer.read().unwrap().username.clone();
        debug!("[peer_actor:{}] disconnect", username);

        self.stream.take();

        if let Err(e) = self
            .client_channel
            .send(ClientOperation::PeerDisconnected(username))
        {
            error!("Failed to send disconnect notification: {}", e);
        }
    }
}

impl Actor for PeerActor {
    type Message = PeerMessage;

    fn handle(&mut self, msg: Self::Message) {
        self.handle_message(msg);
    }

    fn on_start(&mut self) {
        let username = self.peer.read().unwrap().username.clone();

        // Initialize the dispatcher
        self.initialize_dispatcher();

        // Trigger initial read
        if let Some(ref handle) = self.self_handle {
            match handle.send(PeerMessage::ProcessRead) {
                Ok(_) => {},
                Err(e) => error!(
                    "[peer_actor:{}] FAILED to send ProcessRead: {}",
                    username, e
                ),
            }
        } else {
            error!(
                "[peer_actor:{}] self_handle is None! Cannot send ProcessRead",
                username
            );
        }

        // Also do an immediate read as fallback
        self.process_read();
    }

    fn on_stop(&mut self) {
        let username = self.peer.read().unwrap().username.clone();
        trace!("[peer_actor:{}] actor stopping", username);
        self.disconnect();
    }

    fn tick(&mut self) {
        // Periodically check for new data on the socket
        if self.stream.is_some() {
            self.process_read();
        } else {
            let username = self.peer.read().unwrap().username.clone();
            trace!(
                "[peer_actor:{}] tick() called but stream is None",
                username
            );
        }
    }
}
