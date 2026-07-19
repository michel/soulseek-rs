use crate::actor::{Actor, ActorHandle, ConnectionState, OutboundBuffer};
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

use mio::event::Event;
use mio::net::TcpStream;
use mio::{Interest, Registry, Token};
use std::io::{self, Error};
use std::net::TcpStream as StdTcpStream;
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
}

pub struct PeerActor {
    peer: Peer,
    stream: Option<TcpStream>,
    connection_state: ConnectionState,
    reader: MessageReader,
    client_channel: ActorHandle<ClientOperation>,
    self_handle: Option<ActorHandle<PeerMessage>>,
    dispatcher: Option<MessageDispatcher<PeerMessage>>,
    queued_messages: Vec<PeerMessage>,
    outbound: OutboundBuffer,
    own_username: String,
    initiated_outbound: bool,
    established: bool,
    disconnect_reported: bool,
    id: u64,
    io_generation: u64,
}

impl PeerActor {
    #[must_use]
    pub fn new(
        peer: Peer,
        stream: Option<StdTcpStream>,
        reader: Option<MessageReader>,
        client_channel: ActorHandle<ClientOperation>,
        own_username: String,
        id: u64,
    ) -> Self {
        let username = peer.username.clone();
        let initiated_outbound = stream.is_none();
        let stream =
            stream.and_then(|stream| Self::into_mio_stream(&username, stream));
        let connection_state = if stream.is_some() {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };

        Self {
            peer,
            stream,
            connection_state,
            reader: reader.unwrap_or_default(),
            client_channel,
            self_handle: None,
            dispatcher: None,
            queued_messages: Vec::new(),
            outbound: OutboundBuffer::new(),
            own_username,
            initiated_outbound,
            established: false,
            disconnect_reported: false,
            id,
            io_generation: 0,
        }
    }

    fn into_mio_stream(
        username: &str,
        stream: StdTcpStream,
    ) -> Option<TcpStream> {
        if let Err(e) = stream.set_nonblocking(true) {
            error!(
                "[peer:{}] Failed to set accepted stream non-blocking: {}",
                username, e
            );
            return None;
        }
        stream.set_nodelay(true).ok();
        Some(TcpStream::from_std(stream))
    }

    pub fn set_self_handle(&mut self, handle: ActorHandle<PeerMessage>) {
        self.self_handle = Some(handle);
    }

    fn peer_username(&self) -> String {
        self.peer.username.clone()
    }

    fn initialize_dispatcher(&mut self) {
        let mut handlers = Handlers::new();
        handlers.register_handler(FileSearchResponse);
        handlers.register_handler(TransferRequest);
        handlers.register_handler(TransferResponse);
        handlers.register_handler(GetShareFileList);
        handlers.register_handler(UploadFailedHandler);
        handlers.register_handler(PlaceInQueueResponse);
        handlers.register_handler(PeerInit);

        self.dispatcher =
            Some(MessageDispatcher::new("peer".to_string(), handlers));
    }

    fn handle_message(&mut self, msg: PeerMessage) {
        if matches!(self.connection_state, ConnectionState::Connecting { .. })
            && !matches!(&msg, PeerMessage::SetUsername(_))
        {
            self.queued_messages.push(msg);
            return;
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

                if allowed {
                    debug!(
                        "[peer:{}] Transfer allowed, ready to connect with token {:}",
                        username, token
                    );
                    if let Err(e) = self.client_channel.send(
                        ClientOperation::DownloadFromPeer(
                            token,
                            self.peer.clone(),
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
                self.peer.username = username;
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
                    debug!(
                        "Read operation timed out for peer actor {:}:{:}",
                        self.peer.host, self.peer.port
                    );
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
                    let messages = if let Some(ref dispatcher) = self.dispatcher
                    {
                        dispatcher.dispatch(&mut message)
                    } else {
                        warn!("[peer:{}] No dispatcher available!", username);
                        Vec::new()
                    };
                    for msg in messages {
                        self.handle_message(msg);
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
    }

    fn send_message(&mut self, message: Message) {
        let username = self.peer_username();
        if self.stream.is_none() {
            error!("Cannot send message: stream is None");
            return;
        }

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

        self.outbound.push(message.get_buffer());
        self.bump_io_generation();
        self.flush_outbound();
    }

    fn flush_outbound(&mut self) {
        let username = self.peer_username();
        let was_empty = self.outbound.is_empty();
        let Some(stream) = self.stream.as_mut() else {
            return;
        };

        if let Err(e) = self.outbound.flush(stream) {
            error!(
                "[peer:{}] Error writing message: {}. Disconnecting.",
                username, e
            );
            self.disconnect_with_error(e);
        }
        if was_empty != self.outbound.is_empty() {
            self.bump_io_generation();
        }
    }

    fn disconnect_with_error(&mut self, error: Error) {
        let username = self.peer_username();
        debug!("[peer:{}] disconnect", username);

        self.stream.take();
        self.connection_state = ConnectionState::Disconnected;
        self.bump_io_generation();

        if self.disconnect_reported {
            return;
        }
        self.disconnect_reported = true;

        let op = if self.initiated_outbound && !self.established {
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
        self.connection_state = ConnectionState::Disconnected;
        self.bump_io_generation();

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
        let username = self.peer.username.clone();
        let host = self.peer.host.clone();
        let port = self.peer.port;

        let socket_addr =
            format!("{host}:{port}").parse::<std::net::SocketAddr>();

        match socket_addr {
            Ok(addr) => match TcpStream::connect(addr) {
                Ok(stream) => {
                    stream.set_nodelay(true).ok();
                    self.stream = Some(stream);
                    self.connection_state = ConnectionState::Connecting {
                        since: Instant::now(),
                    };
                    self.bump_io_generation();
                    true
                }
                Err(e) => {
                    self.disconnect_with_error(e);
                    false
                }
            },
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

    fn check_connection_timeout(&mut self) {
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
        }
    }

    fn complete_connection(&mut self) {
        if !matches!(self.connection_state, ConnectionState::Connecting { .. })
        {
            return;
        }

        let username = self.peer_username();

        let Some(ref stream) = self.stream else {
            return;
        };

        match stream.take_error() {
            Ok(None) => {
                self.connection_state = ConnectionState::Connected;
                self.bump_io_generation();
                self.on_connection_established();
            }
            Ok(Some(e)) | Err(e) => {
                error!("[peer:{}] Connection failed: {}", username, e);
                self.disconnect_with_error(e);
            }
        }
    }

    fn on_connection_established(&mut self) {
        self.established = true;
        let username = self.peer_username();
        let connection_type = self.peer.connection_type.clone();
        let token = self.peer.token;

        if self.stream.is_none() {
            return;
        }

        if self.initiated_outbound {
            let handshake_msg = match token {
                Some(server_token) => {
                    MessageFactory::build_pierce_firewall_message(server_token)
                }
                None => MessageFactory::build_peer_init_message(
                    &self.own_username,
                    connection_type,
                    0,
                ),
            };
            self.outbound.push(handshake_msg.get_buffer());
            self.bump_io_generation();
            self.flush_outbound();
        }

        self.initialize_dispatcher();

        let queued = std::mem::take(&mut self.queued_messages);
        for msg in queued {
            self.handle_message(msg);
        }

        if self.initiated_outbound
            && let Err(e) = self
                .client_channel
                .send(ClientOperation::PeerConnected(username))
        {
            error!("[peer] failed to notify peer connected: {}", e);
        }

        self.process_read();
    }

    fn io_interest(&self) -> Option<Interest> {
        self.stream.as_ref()?;

        match self.connection_state {
            ConnectionState::Connecting { .. } => Some(Interest::WRITABLE),
            ConnectionState::Connected => {
                let interest = if self.outbound.is_empty() {
                    Interest::READABLE
                } else {
                    Interest::READABLE.add(Interest::WRITABLE)
                };
                Some(interest)
            }
            ConnectionState::Disconnected => None,
        }
    }

    const fn bump_io_generation(&mut self) {
        self.io_generation = self.io_generation.wrapping_add(1);
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
                self.check_connection_timeout();
            }
            ConnectionState::Connected | ConnectionState::Disconnected => {}
        }
    }

    fn handle_io_event(&mut self, _registry: &Registry, event: &Event) {
        if matches!(self.connection_state, ConnectionState::Connecting { .. })
            && event.is_writable()
        {
            self.complete_connection();
        }

        if !matches!(self.connection_state, ConnectionState::Connected) {
            return;
        }

        if event.is_readable() {
            self.process_read();
        }

        if event.is_writable() {
            self.flush_outbound();
        }
    }

    fn io_generation(&self) -> u64 {
        self.io_generation
    }

    fn register_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        let Some(interest) = self.io_interest() else {
            return Ok(false);
        };
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };

        registry.register(stream, token, interest)?;
        Ok(true)
    }

    fn reregister_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        let Some(interest) = self.io_interest() else {
            return Ok(false);
        };
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };

        registry.reregister(stream, token, interest)?;
        Ok(true)
    }

    fn deregister_io(&mut self, registry: &Registry) -> io::Result<()> {
        if let Some(stream) = self.stream.as_mut() {
            registry.deregister(stream)?;
        }
        Ok(())
    }

    fn tick_interval(&self) -> Option<Duration> {
        match self.connection_state {
            ConnectionState::Connecting { .. } => Some(Duration::from_secs(1)),
            ConnectionState::Connected | ConnectionState::Disconnected => None,
        }
    }
}
