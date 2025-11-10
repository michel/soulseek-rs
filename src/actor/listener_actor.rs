use crate::actor::{Actor, ActorSystem};
use crate::client::ClientContext;
use crate::message::{Message, MessageReader};
use crate::peer::{ConnectionType, DownloadPeer, Peer};
use crate::types::Download;
use crate::{debug, error, info, trace, DownloadStatus};

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};

const PEER_INIT_MESSAGE_CODE: u8 = 1;

struct PeerInitData {
    username: String,
    connection_type: ConnectionType,
    token: u32,
}

fn read_peer_init_message(
    stream: &mut TcpStream,
    reader: &mut MessageReader,
) -> io::Result<Message> {
    loop {
        reader.read_from_socket(stream)?;

        if let Ok(Some(msg)) = reader.extract_message() {
            return Ok(msg);
        }
    }
}

fn parse_peer_init_message(mut message: Message) -> Option<PeerInitData> {
    message.set_pointer(4);
    let message_code = message.read_int8();

    if message_code != PEER_INIT_MESSAGE_CODE {
        return None;
    }

    Some(PeerInitData {
        username: message.read_string(),
        connection_type: message.read_string().parse().unwrap(),
        token: message.read_int32(),
    })
}

fn parse_token_from_buffer(buffer: &[u8], username: &str) -> Option<u32> {
    let token_bytes = buffer.get(0..4)?;
    let token = u32::from_le_bytes(
        token_bytes
            .try_into()
            .unwrap_or_else(|_| panic!("[listener:{}] slice with incorrect length, can't extract transfer_token", username)),
    );
    Some(token)
}

fn extract_download_from_buffer(
    reader: &mut MessageReader,
    client_context: &Arc<RwLock<ClientContext>>,
    username: &str,
    peer_ip: &str,
    peer_port: u16,
) -> Option<Download> {
    if reader.buffer_len() == 0 {
        return None;
    }

    let buffer = reader.get_buffer();
    trace!(
        "[listener:{peer_ip}:{peer_port}] reader buffer has {} bytes. {:?}",
        buffer.len(),
        buffer
    );

    let token = parse_token_from_buffer(&buffer, username)?;
    trace!(
        "[listener:{}] got transfer_token: {} from data chunk",
        username,
        token
    );

    let context = client_context.read().unwrap();
    let download = context.download_tokens.get(&token).cloned();

    if download.is_none() {
        let download_tokens: Vec<u32> = context.download_tokens.keys().cloned().collect();
        trace!("[listener:{peer_ip}:{peer_port}] download token not found: {:?}, download tokens: {:?}", token, download_tokens);
    }

    download
}

/// Message types for ConnectionHandlerActor
#[derive(Debug, Clone)]
pub enum ConnectionHandlerMessage {
    /// Process the incoming connection
    Process,
}

/// Actor that handles a single incoming connection
pub struct ConnectionHandlerActor {
    stream: Option<TcpStream>,
    client_context: Arc<RwLock<ClientContext>>,
    own_username: String,
    peer_ip: String,
    peer_port: u16,
}

impl ConnectionHandlerActor {
    pub fn new(
        stream: TcpStream,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) -> Self {
        let peer_addr = stream.peer_addr().unwrap();
        let peer_ip = peer_addr.ip().to_string();
        let peer_port = peer_addr.port();

        Self {
            stream: Some(stream),
            client_context,
            own_username,
            peer_ip,
            peer_port,
        }
    }

    fn handle_peer_connection(
        &self,
        peer: Peer,
        stream: TcpStream,
        reader: MessageReader,
    ) {
        debug!("[listener:{}:{}] connection type is P, reader buffer has {} bytes",
            self.peer_ip, self.peer_port, reader.buffer_len());

        let client_context = self.client_context.read().unwrap();
        if let Some(ref registry) = client_context.peer_registry {
            match registry.register_peer(peer.clone(), stream, Some(reader)) {
                Ok(_) => trace!("[listener] peer actor spawned for: {}", peer.username),
                Err(e) => error!("Failed to spawn peer actor for {:?}: {:?}", peer.username, e),
            }
        } else {
            error!("PeerRegistry not initialized");
        }
    }

    fn handle_file_connection(
        &self,
        peer: Peer,
        stream: TcpStream,
        mut reader: MessageReader,
        token: u32,
    ) {
        trace!(
            "[client] DownloadFromPeer token: {} peer: {:?}",
            token,
            peer
        );

        // Set stream to blocking mode - DownloadPeer expects blocking I/O
        if let Err(e) = stream.set_nonblocking(false) {
            error!(
                "Failed to set stream to blocking mode for {}:{} (token: {}) - Error: {}",
                peer.host, peer.port, token, e
            );
            return;
        }

        let download = extract_download_from_buffer(
            &mut reader,
            &self.client_context,
            &peer.username,
            &self.peer_ip,
            self.peer_port,
        );

        let download_peer = DownloadPeer::new(
            format!("{}:direct", peer.username),
            peer.host.clone(),
            peer.port,
            token,
            true,
            self.own_username.clone(),
        );

        match download_peer.download_file(
            self.client_context.clone(),
            download,
            Some(stream),
        ) {
            Ok((download, filename)) => {
                download.sender.send(DownloadStatus::Completed).unwrap();
                info!(
                    "Successfully downloaded {} bytes to {}",
                    download.size, filename
                );
            }
            Err(e) => {
                error!(
                    "Failed to download file from {}:{} (token: {}) - Error: {}",
                    peer.host, peer.port, token, e
                );
            }
        }
    }

    fn process_connection(&mut self) {
        let Some(mut stream) = self.stream.take() else {
            error!("[listener] stream already taken");
            return;
        };

        let mut reader = MessageReader::new();

        let Ok(message) = read_peer_init_message(&mut stream, &mut reader) else {
            error!(
                "[listener:{}:{}] Failed to read peer init message",
                self.peer_ip, self.peer_port
            );
            return;
        };

        let Some(init_data) = parse_peer_init_message(message) else {
            error!("[listener:{}:{}] Invalid or unknown peer init message",
                self.peer_ip, self.peer_port);
            return;
        };

        debug!(
            "[listener:{}:{}] peerInit (0)  username: {} connection_type: {} token: {}",
            self.peer_ip, self.peer_port, init_data.username, init_data.connection_type, init_data.token
        );

        let peer = Peer::new(
            format!("{}:direct", init_data.username),
            init_data.connection_type.clone(),
            self.peer_ip.clone(),
            self.peer_port.into(),
            None,
            0,
            0,
            0,
        );

        match init_data.connection_type {
            ConnectionType::P => self.handle_peer_connection(peer, stream, reader),
            ConnectionType::F => self.handle_file_connection(peer, stream, reader, init_data.token),
            ConnectionType::D => {
                debug!("[listener:{}:{}] connection type is D, not supported yet, closing connection.",
                    self.peer_ip, self.peer_port);
            }
        }
    }
}

impl Actor for ConnectionHandlerActor {
    type Message = ConnectionHandlerMessage;

    fn handle(&mut self, msg: Self::Message) {
        match msg {
            ConnectionHandlerMessage::Process => self.process_connection(),
        }
    }

    fn on_start(&mut self) {
        trace!("[connection_handler:{}:{}] starting", self.peer_ip, self.peer_port);
        self.process_connection();
    }
}

/// Message types for ListenerActor
#[derive(Debug, Clone)]
pub enum ListenerMessage {
    /// Stop the listener
    Stop,
}

/// Actor that accepts incoming TCP connections
pub struct ListenerActor {
    listener: Option<TcpListener>,
    client_context: Arc<RwLock<ClientContext>>,
    own_username: String,
    actor_system: Arc<ActorSystem>,
}

impl ListenerActor {
    pub fn new(
        port: u32,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
        actor_system: Arc<ActorSystem>,
    ) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
        listener.set_nonblocking(true)?;

        info!("[listener] starting listener on port {port}");

        Ok(Self {
            listener: Some(listener),
            client_context,
            own_username,
            actor_system,
        })
    }
}

impl Actor for ListenerActor {
    type Message = ListenerMessage;

    fn handle(&mut self, msg: Self::Message) {
        match msg {
            ListenerMessage::Stop => {
                info!("[listener] stopping listener");
                self.listener = None;
            }
        }
    }

    fn tick(&mut self) {
        let Some(ref listener) = self.listener else {
            return;
        };

        match listener.accept() {
            Ok((stream, _addr)) => {
                trace!("[listener] accepted new connection");
                let actor = ConnectionHandlerActor::new(
                    stream,
                    self.client_context.clone(),
                    self.own_username.clone(),
                );
                self.actor_system.spawn(actor);
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // No incoming connections, this is normal for non-blocking
            }
            Err(e) => {
                error!("[listener] Failed to accept connection: {}", e);
            }
        }
    }
}
