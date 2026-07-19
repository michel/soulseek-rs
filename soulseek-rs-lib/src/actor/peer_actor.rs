use crate::actor::{Actor, ActorHandle, ConnectionState};
use crate::client::ClientOperation;
use crate::dispatcher::MessageDispatcher;
use crate::message::peer::{
    FileSearchResponse, GetShareFileList, PeerInit, PlaceInQueueResponse,
    QueueUploadHandler, SharedDirectory, SharedFileListResponseHandler,
    TransferRequest, TransferResponse, UploadFailedHandler,
};
use crate::message::server::MessageFactory;
use crate::message::{Handlers, Message, MessageReader, MessageType};
use crate::peer::Peer;
use crate::types::{Download, SearchResult, Transfer};
use crate::utils::lock::RwLockExt;
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
    /// A peer queued one of our shared files for download (they sent us code 43).
    IncomingQueueUpload(String),
    /// A peer asked to browse our shared files (they sent us code 4).
    ShareListRequested,
    /// A peer we are browsing sent us their shared-file listing (code 5).
    ShareListReceived(Vec<SharedDirectory>),
    /// Offer the queued file to that peer: send an upload TransferRequest.
    ServeUpload {
        token: u32,
        filename: String,
        size: u64,
    },
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
    own_username: String,
    /// True when we initiated this connection (no stream supplied at
    /// construction), so we must send a `PeerInit` once connected.
    outbound: bool,
    /// Set once the connection is up. A pre-established failure on an outbound
    /// connection means the peer is unreachable (likely firewalled).
    established: bool,
    /// Set once this actor has reported a terminal outcome (disconnect or
    /// connect-failure). Guards against a second notification — e.g. when a
    /// replaced actor is later stopped — which would otherwise alias and evict
    /// a newer actor registered under the same username.
    disconnect_reported: bool,
    /// Registry-assigned unique id, echoed in terminal notifications so the
    /// client evicts only this actor and never a newer namesake.
    id: u64,
    /// Transfer tokens for uploads we are serving to this peer. A TransferResponse
    /// for one of these is our upload being accepted, not a download offer.
    serving_tokens: std::collections::HashSet<u32>,
}

impl PeerActor {
    #[must_use]
    pub fn new(
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
        client_channel: Sender<ClientOperation>,
        own_username: String,
        id: u64,
    ) -> Self {
        let outbound = stream.is_none();
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
            own_username,
            outbound,
            established: false,
            disconnect_reported: false,
            id,
            serving_tokens: std::collections::HashSet::new(),
        }
    }

    pub fn set_self_handle(&mut self, handle: ActorHandle<PeerMessage>) {
        self.self_handle = Some(handle);
    }

    fn peer_username(&self) -> String {
        match self.peer.read_safe() {
            Ok(p) => p.username.clone(),
            Err(e) => {
                error!("[peer_actor] peer lock poisoned: {}", e);
                "<unknown>".to_string()
            }
        }
    }

    fn peer_snapshot(&self) -> Option<Peer> {
        match self.peer.read_safe() {
            Ok(p) => Some(p.clone()),
            Err(e) => {
                error!("[peer_actor] peer lock poisoned: {}", e);
                None
            }
        }
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
        handlers.register_handler(QueueUploadHandler);
        handlers.register_handler(SharedFileListResponseHandler);
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

        for msg in &messages {
            self.handle_message(msg.clone());
        }
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
                if let Err(e) = self
                    .client_channel
                    .send(ClientOperation::SearchResult(file_search))
                {
                    error!(
                        "[peer:{}] failed to forward search result: {}",
                        self.peer_username(),
                        e
                    );
                }
            }
            PeerMessage::TransferRequest(transfer) => {
                let username = self.peer_username();
                debug!(
                    "[peer:{}] TransferRequest for {}",
                    username, transfer.token
                );

                if let Err(e) = self.client_channel.send(
                    ClientOperation::UpdateDownloadTokens(
                        transfer.clone(),
                        username.clone(),
                    ),
                ) {
                    error!(
                        "[peer:{}] failed to send UpdateDownloadTokens: {}",
                        username, e
                    );
                }

                let transfer_response =
                    MessageFactory::build_transfer_response_message(transfer);

                if let Some(ref handle) = self.self_handle
                    && let Err(e) =
                        handle.send(PeerMessage::SendMessage(transfer_response))
                {
                    error!(
                        "[peer:{}] Failed to send TransferResponse message: {}",
                        username, e
                    );
                }
            }
            PeerMessage::TransferResponse {
                token,
                allowed,
                reason,
            } => {
                let username = self.peer_username();
                debug!(
                    "[peer:{}] transfer response token: {} allowed: {}",
                    username, token, allowed
                );

                // If this token is one of our uploads, the peer just accepted
                // our offer — start streaming. This leaves the download path
                // (every other token) byte-for-byte unchanged.
                if self.serving_tokens.remove(&token) {
                    if allowed {
                        let _ = self
                            .client_channel
                            .send(ClientOperation::StartUpload { token });
                    }
                    return;
                }

                if allowed {
                    debug!(
                        "[peer:{}] Transfer allowed, ready to connect with token {:}",
                        username, token
                    );
                    let Some(peer_snapshot) = self.peer_snapshot() else {
                        return;
                    };
                    if let Err(e) = self.client_channel.send(
                        ClientOperation::DownloadFromPeer(
                            token,
                            peer_snapshot,
                            allowed,
                        ),
                    ) {
                        error!(
                            "[peer:{}] failed to send DownloadFromPeer: {}",
                            username, e
                        );
                    }
                } else if let Some(reason_text) = reason {
                    debug!(
                        "[peer:{}] Transfer rejected: {} - token {}, waiting for TransferRequest...",
                        username, reason_text, token
                    );
                }
            }
            PeerMessage::PlaceInQueueResponse { filename, place } => {
                let username = self.peer_username();
                debug!(
                    "[peer:{}] Place in queue response - file: {}, place: {}",
                    username, filename, place
                );
                if let Err(e) = self.client_channel.send(
                    ClientOperation::PlaceInQueueUpdate {
                        username: username.clone(),
                        filename,
                        place,
                    },
                ) {
                    error!(
                        "[peer:{}] failed to forward PlaceInQueueUpdate: {}",
                        username, e
                    );
                }
            }
            PeerMessage::SetUsername(username) => {
                trace!(
                    "[peer:{}] SetUsername: {}",
                    self.peer_username(),
                    username
                );
                match self.peer.write_safe() {
                    Ok(mut p) => p.username = username,
                    Err(e) => {
                        error!("[peer_actor] SetUsername write: {}", e);
                    }
                }
            }
            PeerMessage::QueueUpload(filename) => {
                let message =
                    MessageFactory::build_queue_upload_message(&filename);
                self.send_message(message);
            }
            PeerMessage::IncomingQueueUpload(filename) => {
                // A peer wants to download one of our shared files. Ask the
                // client (which owns the shares) to prepare the upload.
                let requester_key = self.peer_username();
                if let Err(e) =
                    self.client_channel.send(ClientOperation::QueueUpload {
                        requester_key,
                        filename,
                    })
                {
                    error!("[peer_actor] forward IncomingQueueUpload: {}", e);
                }
            }
            PeerMessage::ServeUpload {
                token,
                filename,
                size,
            } => {
                self.serving_tokens.insert(token);
                let message = MessageFactory::build_upload_transfer_request(
                    &filename, token, size,
                );
                self.send_message(message);
            }
            PeerMessage::ShareListRequested => {
                let requester_key = self.peer_username();
                if let Err(e) = self
                    .client_channel
                    .send(ClientOperation::ShareListRequested { requester_key })
                {
                    error!("[peer_actor] forward ShareListRequested: {}", e);
                }
            }
            PeerMessage::ShareListReceived(directories) => {
                let username = self.peer_username();
                if let Err(e) =
                    self.client_channel.send(ClientOperation::BrowseResult {
                        username,
                        directories,
                    })
                {
                    error!("[peer_actor] forward BrowseResult: {}", e);
                }
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
                if let Err(e) = self
                    .client_channel
                    .send(ClientOperation::UploadFailed(username, filename))
                {
                    error!(
                        "[peer_actor] failed to forward UploadFailed: {}",
                        e
                    );
                }
            }
        }
    }

    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages();
        }

        {
            let Some(stream) = self.stream.as_mut() else {
                return;
            };

            match self.reader.read_from_socket(stream) {
                Ok(()) => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    if let Ok(peer_lock) = self.peer.read_safe() {
                        debug!(
                            "Read operation timed out for peer actor {:}:{:}",
                            peer_lock.host, peer_lock.port
                        );
                    }
                }
                Err(e) => {
                    let username = self.peer_username();
                    error!(
                        "[peer:{}] Error reading from peer: {} (kind: {:?}). Disconnecting.",
                        username,
                        e,
                        e.kind()
                    );
                    self.disconnect_with_error(e);
                    return;
                }
            }
        }
        self.extract_and_process_messages();
    }

    fn extract_and_process_messages(&mut self) {
        let username = self.peer_username();
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
                                u32::from(message.get_message_code())
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
        let username = self.peer_username();
        let Some(stream) = self.stream.as_mut() else {
            error!("Cannot send message: stream is None");
            return;
        };

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
        let username = self.peer_username();
        debug!("[peer:{}] disconnect", username);

        self.stream.take();

        if self.disconnect_reported {
            return;
        }
        self.disconnect_reported = true;

        // A direct outbound connection that never established means the peer is
        // unreachable (likely firewalled): signal a connect failure so the
        // client can fall back to server-brokered connect. Anything else is a
        // normal disconnect.
        let op = if self.outbound && !self.established {
            ClientOperation::PeerConnectFailed(self.id, username)
        } else {
            ClientOperation::PeerDisconnected(
                self.id,
                username,
                Some(error.into()),
            )
        };

        if let Err(e) = self.client_channel.send(op) {
            error!("Failed to send disconnect notification: {}", e);
        }
    }
    fn disconnect(&mut self) {
        let username = self.peer_username();
        debug!("[peer:{}] disconnect", username);

        self.stream.take();

        if self.disconnect_reported {
            return;
        }
        self.disconnect_reported = true;

        if let Err(e) = self
            .client_channel
            .send(ClientOperation::PeerDisconnected(self.id, username, None))
        {
            error!("Failed to send disconnect notification: {}", e);
        }
    }

    fn initiate_connection(&mut self) -> bool {
        let (username, host, port) = match self.peer.read_safe() {
            Ok(peer) => (peer.username.clone(), peer.host.clone(), peer.port),
            Err(e) => {
                error!("[peer_actor] initiate_connection peer lock: {}", e);
                return false;
            }
        };

        let socket_addr =
            format!("{host}:{port}").parse::<std::net::SocketAddr>();

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

        let username = self.peer_username();

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
        let (username, connection_type, token) = match self.peer.read_safe() {
            Ok(peer) => (
                peer.username.clone(),
                peer.connection_type.clone(),
                peer.token,
            ),
            Err(e) => {
                error!(
                    "[peer_actor] on_connection_established peer lock: {}",
                    e
                );
                return;
            }
        };

        self.established = true;

        let Some(ref mut stream) = self.stream else {
            return;
        };

        // Connections we initiated must announce themselves; inbound peers
        // already sent us theirs, so we stay silent for them. A direct dial
        // (resolved via GetPeerAddress, token None) sends a PeerInit; a dial we
        // make because the server asked us to (a brokered ConnectToPeer carries
        // the server's token) must instead PierceFirewall with that token so
        // the remote peer can correlate its own request. `get_buffer` prepends
        // the length prefix the peer's MessageReader expects.
        if self.outbound {
            let handshake = match token {
                Some(server_token) => {
                    MessageFactory::build_pierce_firewall_message(server_token)
                }
                None => MessageFactory::build_peer_init_message(
                    &self.own_username,
                    connection_type,
                    0,
                ),
            };
            if let Err(e) = stream.write_all(&handshake.get_buffer()) {
                error!(
                    "[peer:{}] Failed to send outbound handshake: {}",
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

        // Tell the client the control connection is live so any downloads
        // queued for this peer can now be requested over a handshaken stream.
        if self.outbound {
            let _ = self
                .client_channel
                .send(ClientOperation::PeerConnected(username));
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
        let username = self.peer_username();
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
