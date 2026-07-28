use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::client::{ClientContext, ClientOperation};

use crate::message::{Message, MessageReader};
use crate::peer::{ConnectionType, DownloadPeer, Peer};
use crate::types::Download;
use crate::utils::lock::RwLockExt;
use crate::{DownloadStatus, debug, error, info, trace};

/// How long to wait before accepting again after a failure.
///
/// The failure worth pausing for is running out of file descriptors, which a
/// busy search can reach: it does not clear by the next instruction, and
/// retrying flat out spins a core and floods the log.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

const PEER_INIT_MESSAGE_CODE: u8 = 1;

/// How long an accepted peer gets to send its peer-init handshake.
const PEER_INIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct ConnectionContext {
    client_sender: Sender<ClientOperation>,
    client_context: Arc<RwLock<ClientContext>>,
    own_username: String,
}

struct PeerInitData {
    username: String,
    connection_type: ConnectionType,
    token: u32,
}

fn read_peer_init_message(
    stream: &mut TcpStream,
    reader: &mut MessageReader,
) -> io::Result<Message> {
    // An untrusted peer gets a bounded handshake. Without this a peer that
    // connects and stays silent parks this read forever, pinning a thread that
    // owes us a peer init.
    stream.set_read_timeout(Some(PEER_INIT_TIMEOUT))?;
    let message = loop {
        reader.read_from_socket(stream)?;
        if let Ok(Some(msg)) = reader.extract_message() {
            break msg;
        }
    };

    // Whatever this connection turns into reads on its own terms from here.
    stream.set_read_timeout(None)?;
    Ok(message)
}

fn parse_peer_init_message(mut message: Message) -> Option<PeerInitData> {
    message.set_pointer(4);
    let message_code = message.read_int8();

    if message_code != PEER_INIT_MESSAGE_CODE {
        return None;
    }

    let username = message.read_string();
    // An untrusted peer can send any connection-type string; an unknown value
    // must be rejected, not panic the listener accept loop.
    let connection_type = message.read_string().parse().ok()?;
    Some(PeerInitData {
        username,
        connection_type,
        token: message.read_int32(),
    })
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
    let token = u32::from_le_bytes(buffer.get(0..4)?.try_into().ok()?);
    trace!(
        "[listener:{}] got transfer_token: {} from data chunk",
        username, token
    );

    let context = match client_context.read_safe() {
        Ok(c) => c,
        Err(e) => {
            error!("[listener] client context lock: {}", e);
            return None;
        }
    };
    let download = context.get_download_by_token(token).cloned();

    if download.is_none() {
        let download_tokens = context.get_download_tokens();
        trace!(
            "[listener:{peer_ip}:{peer_port}] download token not found: {:?}, download tokens: {:?}",
            token, download_tokens
        );
    }

    download
}

fn handle_peer_connection(
    peer: Peer,
    stream: TcpStream,
    reader: MessageReader,
    context: &ConnectionContext,
) {
    // The peer actor multiplexes socket reads with its mailbox on a single
    // thread: `tick()` reads the socket, but outgoing messages (e.g. a queued
    // QueueUpload for a download) are delivered through the mailbox between
    // ticks. A blocking socket would park `tick()` inside `read` whenever the
    // peer is idle, starving the mailbox and stalling downloads. Match the
    // outbound path and drive this connection non-blocking.
    if let Err(e) = stream.set_nonblocking(true) {
        error!("[listener] failed to set peer stream non-blocking: {}", e);
        return;
    }
    stream.set_nodelay(true).ok();

    let client_context = match context.client_context.read_safe() {
        Ok(c) => c,
        Err(e) => {
            error!("[listener] handle_peer_connection lock: {}", e);
            return;
        }
    };
    if let Some(ref registry) = client_context.peer_registry {
        match registry.register_peer(peer.clone(), Some(stream), Some(reader)) {
            Ok(_) => (),
            Err(e) => {
                error!(
                    "Failed to spawn peer actor for {:?}: {:?}",
                    peer.username, e
                );
            }
        }
    } else {
        error!("PeerRegistry not initialized");
    }
}

fn handle_file_connection(
    peer: Peer,
    stream: TcpStream,
    mut reader: MessageReader,
    token: u32,
    context: &ConnectionContext,
    peer_ip: &str,
    peer_port: u16,
) {
    trace!(
        "[client] DownloadFromPeer token: {} peer: {:?}",
        token, peer
    );

    let download = extract_download_from_buffer(
        &mut reader,
        &context.client_context,
        &peer.username,
        peer_ip,
        peer_port,
    );
    let failure_token = download.as_ref().map(|d| d.token);

    let download_peer = DownloadPeer::new(
        format!("{}:direct", peer.username),
        peer.host.clone(),
        peer.port,
        token,
        true,
        context.own_username.clone(),
    );

    match download_peer.download_file(
        context.client_context.clone(),
        download,
        Some(stream),
    ) {
        Ok((download, filename)) => {
            let _ = download.sender.send(DownloadStatus::Completed);
            match context.client_context.write_safe() {
                Ok(mut ctx) => ctx.update_download_with_status(
                    download.token,
                    DownloadStatus::Completed,
                ),
                Err(e) => {
                    error!("[listener] handle_file_connection write: {}", e);
                }
            }
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
            // A failed incoming transfer (e.g. a truncated/incomplete download)
            // must not leave the download stuck as Queued/InProgress forever.
            if let Some(failure_token) = failure_token {
                match context.client_context.write_safe() {
                    Ok(mut ctx) => ctx.update_download_with_status(
                        failure_token,
                        DownloadStatus::Failed(Some(e.to_string())),
                    ),
                    Err(e) => {
                        error!(
                            "[listener] handle_file_connection fail write: {}",
                            e
                        );
                    }
                }
            }
        }
    }
}

/// Handle a `PierceFirewall` (peer init code 0): a peer we asked the server to
/// broker (because a direct connection failed) connecting back to us. It quotes
/// the correlation token we registered; we register the stream as its P control
/// connection and tell the client the connection is live.
fn handle_pierce_firewall(
    mut message: Message,
    stream: TcpStream,
    reader: MessageReader,
    context: &ConnectionContext,
    peer_ip: &str,
    peer_port: u16,
) {
    message.set_pointer(5); // skip length prefix (4) + int8 code (1)
    let token = message.read_int32();

    let username = match context.client_context.write_safe() {
        Ok(mut ctx) => ctx.take_pending_connect(token),
        Err(e) => {
            error!("[listener] pierce firewall lock: {}", e);
            return;
        }
    };
    let Some(username) = username else {
        debug!(
            "[listener:{peer_ip}:{peer_port}] PierceFirewall token {token} is not pending; ignoring"
        );
        return;
    };

    let peer = Peer::new(
        username.clone(),
        ConnectionType::P,
        peer_ip.to_string(),
        peer_port.into(),
        None,
        0,
        0,
        0,
    );
    handle_peer_connection(peer, stream, reader, context);

    // Inbound peers don't self-announce, so nudge the client to flush any
    // downloads queued for this now-connected peer.
    let _ = context
        .client_sender
        .send(ClientOperation::PeerConnected(username));
}

fn handle_incoming_connection(
    mut stream: TcpStream,
    context: ConnectionContext,
) {
    let Ok(peer_addr) = stream.peer_addr() else {
        error!("[listener] failed to get peer address");
        return;
    };

    let peer_ip = peer_addr.ip().to_string();
    let peer_port = peer_addr.port();
    let mut reader = MessageReader::new();

    // A peer that dials and then goes away is routine, not an error.
    let message = match read_peer_init_message(&mut stream, &mut reader) {
        Ok(message) => message,
        Err(e) => {
            debug!(
                "[listener:{peer_ip}:{peer_port}] no peer init message: {e}"
            );
            return;
        }
    };

    // A firewalled peer brokered through the server connects back with a
    // PierceFirewall (code 0) instead of a PeerInit (code 1).
    if message.get_message_code() == 0 {
        handle_pierce_firewall(
            message, stream, reader, &context, &peer_ip, peer_port,
        );
        return;
    }

    let Some(init_data) = parse_peer_init_message(message) else {
        error!(
            "[listener:{peer_ip}:{peer_port}] Invalid or unknown peer init message"
        );
        return;
    };

    debug!(
        "[listener:{peer_ip}:{peer_port}] peerInit (0)  username: {} connection_type: {} token: {}",
        init_data.username, init_data.connection_type, init_data.token
    );

    let peer = Peer::new(
        format!("{}:direct", init_data.username),
        init_data.connection_type.clone(),
        peer_ip.clone(),
        peer_port.into(),
        None,
        0,
        0,
        0,
    );

    match init_data.connection_type {
        ConnectionType::P => {
            handle_peer_connection(peer, stream, reader, &context);
        }

        ConnectionType::F => handle_file_connection(
            peer,
            stream,
            reader,
            init_data.token,
            &context,
            &peer_ip,
            peer_port,
        ),
        ConnectionType::D => {
            debug!(
                "[listener:{peer_ip}:{peer_port}] connection type is D, not supported yet, closing connection. "
            );
        }
    }
}

pub struct Listen {}

impl Listen {
    /// Take the peer-listening socket, falling back to a port the operating
    /// system picks when the configured one is already held.
    ///
    /// Several clients on one machine share one configured port, and losing
    /// the race for it must not cost a session its listener: an unreachable
    /// client is one that never receives search responses or transfers. The
    /// caller advertises [`TcpListener::local_addr`], so peers are always told
    /// the port that was really bound.
    pub fn bind(port: u16) -> io::Result<TcpListener> {
        match TcpListener::bind(("0.0.0.0", port)) {
            Err(e) if e.kind() == io::ErrorKind::AddrInUse && port != 0 => {
                info!(
                    "[listener] port {port} is taken, falling back to an \
                     ephemeral port"
                );
                TcpListener::bind(("0.0.0.0", 0))
            }
            other => other,
        }
    }

    pub fn serve(
        listener: &TcpListener,
        client_sender: Sender<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        info!("[listener] listening on {:?}", listener.local_addr());

        let context = ConnectionContext {
            client_sender,
            client_context,
            own_username,
        };

        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                error!(
                    "[listener] Failed to accept connection: {}",
                    stream.unwrap_err()
                );
                // Running out of file descriptors does not clear by the next
                // instruction, and retrying flat out turns one exhausted
                // moment into a spinning core and thousands of identical log
                // lines. Pause long enough for something to be released.
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            };

            let context = context.clone();
            // One thread per connection: the peer-init handshake blocks, and a
            // peer that is slow to send one must not stop us accepting anybody
            // else — a wedged accept loop makes us unreachable to every peer.
            thread::spawn(move || handle_incoming_connection(stream, context));
        }
    }
}
