use crate::actor::{Actor, ActorHandle, ConnectionState};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::peer::{
    FileSearchResponse, GetShareFileList, PeerInit, PlaceInQueueResponse,
    TransferRequest, TransferResponse, UploadFailedHandler,
};
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader, MessageType};
use crate::peer::Peer;
use crate::types::{Download, SearchResult, Transfer};
use crate::{debug, error, trace, warn};

use std::io::{self, Error, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum PeerMessage {
    SendMessage(Message),
    FileSearchResult(SearchResult),
    TransferRequest(Transfer),
    UploadFailed(String, String),
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
    QueueUpload(String),
    RequestTransfer(Download),
    ProcessRead,
}

pub struct PeerActor {
    peer: Arc<RwLock<Peer>>,
    stream: Option<TcpStream>,
    connection_state: ConnectionState,
    reader: MessageReader,
    client_channel: Sender<ClientOperation>,
    self_handle: Option<ActorHandle<PeerMessage>>,
    dispatcher: Option<MessageDispatcher<PeerMessage>>,
    dispatcher_receiver: Option<Receiver<PeerMessage>>,
    queued_messages: Vec<PeerMessage>,
}

impl PeerActor {
    pub fn new(
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
        client_channel: Sender<ClientOperation>,
    ) -> Self {
        let connection_state = if stream.is_some() {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };

        Self {
            peer: Arc::new(RwLock::new(peer)),
            stream,
            connection_state,
            reader: reader.unwrap_or_default(),
            client_channel,
            self_handle: None,
            dispatcher: None,
            dispatcher_receiver: None,
            queued_messages: Vec::new(),
        }
    }

    pub fn set_self_handle(&mut self, handle: ActorHandle<PeerMessage>) {
        self.self_handle = Some(handle);
    }

    fn initialize_dispatcher(&mut self) {
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
            "peer".to_string(),
            dispatcher_sender,
            handlers,
        ));
    }

    fn process_dispatcher_messages(&mut self) {
        let messages: Vec<PeerMessage> = self
            .dispatcher_receiver
            .as_ref()
            .map_or_else(Vec::new, |receiver| {
                let mut msgs = Vec::new();
                while let Ok(msg) = receiver.try_recv() {
                    msgs.push(msg);
                }
                msgs
            });

        messages
            .iter()
            .for_each(|msg| self.handle_message(msg.clone()));
    }

    fn handle_message(&mut self, msg: PeerMessage) {
        if matches!(self.connection_state, ConnectionState::Connecting { .. }) {
            match &msg {
                PeerMessage::SetUsername(_) | PeerMessage::ProcessRead => {}
                _ => {
                    self.queued_messages.push(msg);
                    return;
                }
            }
        }

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
                    "[peer:{}] TransferRequest for {}",
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
                        error!("[peer:{}] Failed to send TransferResponse message: {}", username, e);
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
                    "[peer:{}] transfer response token: {} allowed: {}",
                    username, token, allowed
                );

                if !allowed {
                    if let Some(reason_text) = reason {
                        debug!(
                                    "[peer:{}] Transfer rejected: {} - token {}, waiting for TransferRequest...",
                                    username, reason_text, token
                                );
                    }
                } else {
                    debug!(
                                "[peer:{}] Transfer allowed, ready to connect with token {:}",
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
                    "[peer:{}] Place in queue response - file: {}, place: {}",
                    username, filename, place
                );
            }
            PeerMessage::SetUsername(username) => {
                trace!(
                    "[peer:{}] SetUsername: {}",
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
            PeerMessage::UploadFailed(username, filename) => {
                self.client_channel
                    .send(ClientOperation::UploadFailed(username, filename))
                    .unwrap();
            }
        }
    }

    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages();
        }

        {
            let stream = match self.stream.as_mut() {
                Some(s) => s,
                None => return,
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
                        "[peer:{}] Error reading from peer: {} (kind: {:?}). Disconnecting.",
                        username, e, e.kind()
                    );
                    self.disconnect_with_error(e);
                    return;
                }
            }
        }
        self.extract_and_process_messages();
    }

    fn extract_and_process_messages(&mut self) {
        let username = self.peer.read().unwrap().username.clone();
        let mut extracted_count = 0;
        loop {
            match self.reader.extract_message() {
                Ok(Some(mut message)) => {
                    extracted_count += 1;
                    trace!(
                        "[peer:{}] ← Message #{}: {:?}",
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
                        warn!("[peer:{}] No dispatcher available!", username);
                    }
                }
                Err(e) => {
                    warn!(
                        "[peer:{}] Error extracting message: {}. Disconnecting peer.",
                        username, e
                    );
                    self.disconnect_with_error(e);
                    return;
                }
                Ok(None) => {
                    break;
                }
            }
        }

        self.process_dispatcher_messages();
    }

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
            "[peer:{}] ➡ {:?}",
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
                "[peer:{}] Error writing message: {}. Disconnecting.",
                username, e
            );
            self.disconnect_with_error(e);
            return;
        }

        if let Err(e) = stream.flush() {
            error!(
                "[peer:{}] Error flushing stream: {}. Disconnecting.",
                username, e
            );
            self.disconnect_with_error(e);
        }
    }

    fn disconnect_with_error(&mut self, error: Error) {
        let username = self.peer.read().unwrap().username.clone();
        debug!("[peer:{}] disconnect", username);

        self.stream.take();

        if let Err(e) = self.client_channel.send(
            ClientOperation::PeerDisconnected(username, Some(error.into())),
        ) {
            error!("Failed to send disconnect notification: {}", e);
        }
    }
    fn disconnect(&mut self) {
        let username = self.peer.read().unwrap().username.clone();
        debug!("[peer:{}] disconnect", username);

        self.stream.take();

        if let Err(e) = self
            .client_channel
            .send(ClientOperation::PeerDisconnected(username, None))
        {
            error!("Failed to send disconnect notification: {}", e);
        }
    }

    fn initiate_connection(&mut self) -> bool {
        let peer = self.peer.read().unwrap();
        let username = peer.username.clone();
        let host = peer.host.clone();
        let port = peer.port;
        drop(peer);

        let socket_addr =
            format!("{}:{}", host, port).parse::<std::net::SocketAddr>();

        match socket_addr {
            Ok(addr) => {
                // Use connect_timeout to prevent blocking the thread for too long
                let timeout = Duration::from_secs(5);
                match TcpStream::connect_timeout(&addr, timeout) {
                    Ok(stream) => {
                        if let Err(e) = stream.set_nonblocking(true) {
                            error!(
                                "[peer:{}] Failed to set non-blocking: {}",
                                username, e
                            );
                            self.disconnect_with_error(e);
                            return false;
                        }
                        stream.set_nodelay(true).ok();
                        self.stream = Some(stream);
                        self.connection_state = ConnectionState::Connecting {
                            since: Instant::now(),
                        };
                        true
                    }
                    Err(e) => {
                        self.disconnect_with_error(e);
                        false
                    }
                }
            }
            Err(e) => {
                error!(
                    "[peer:{}] Invalid socket address {}:{} - {}",
                    username, host, port, e
                );
                self.disconnect_with_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    e,
                ));
                false
            }
        }
    }

    fn check_connection_status(&mut self) {
        let ConnectionState::Connecting { since } = self.connection_state
        else {
            return;
        };

        let username = self.peer.read().unwrap().username.clone();

        if since.elapsed() > Duration::from_secs(20) {
            error!("[peer:{}] Connection timeout after 20 seconds", username);
            self.disconnect_with_error(io::Error::new(
                io::ErrorKind::TimedOut,
                "Connection timeout",
            ));
            return;
        }

        let Some(ref stream) = self.stream else {
            return;
        };

        match stream.peer_addr() {
            Ok(_) => {
                self.connection_state = ConnectionState::Connected;
                self.on_connection_established();
            }
            Err(ref e) if e.kind() == io::ErrorKind::NotConnected => {}
            Err(e) => {
                error!("[peer:{}] Connection failed: {}", username, e);
                self.disconnect_with_error(e);
            }
        }
    }

    fn on_connection_established(&mut self) {
        let peer = self.peer.read().unwrap();
        let username = peer.username.clone();
        let token = peer.token;
        drop(peer);

        let Some(ref mut stream) = self.stream else {
            return;
        };

        if let Some(token) = token {
            let handshake_msg = MessageFactory::build_watch_user(token);
            if let Err(e) = stream.write_all(&handshake_msg.get_data()) {
                error!(
                    "[peer:{}] Failed to send watch user handshake: {}",
                    username, e
                );
                self.disconnect_with_error(e);
                return;
            }
        }

        self.initialize_dispatcher();

        let queued = std::mem::take(&mut self.queued_messages);
        for msg in queued {
            self.handle_message(msg);
        }

        if let Some(ref handle) = self.self_handle {
            handle.send(PeerMessage::ProcessRead).ok();
        }

        self.process_read();
    }
}

impl Actor for PeerActor {
    type Message = PeerMessage;

    fn handle(&mut self, msg: Self::Message) {
        self.handle_message(msg);
    }

    fn on_start(&mut self) {
        if self.stream.is_none() {
            self.initiate_connection();
        } else {
            self.connection_state = ConnectionState::Connected;
            self.on_connection_established();
        }
    }

    fn on_stop(&mut self) {
        let username = self.peer.read().unwrap().username.clone();
        trace!("[peer:{}] actor stopping", username);
        self.disconnect();
    }

    fn tick(&mut self) {
        match self.connection_state {
            ConnectionState::Connecting { .. } => {
                self.check_connection_status();
            }
            ConnectionState::Connected => {
                if self.stream.is_some() {
                    self.process_read();
                }
            }
            ConnectionState::Disconnected => {}
        }
    }
}
