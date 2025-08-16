use crate::client::ClientOperation;
use crate::message::MessageReader;
use crate::peer::{DefaultPeer, Peer};
use crate::{debug, error, info, warn};
use std::io::{self};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

pub struct Listen {}

impl Listen {
    pub fn start(port: u32, client_channel: Sender<ClientOperation>) {
        info!("Starting peer listener on port {}", port);
        let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)) {
            Ok(l) => l,
            Err(e) => {
                error!("Could not bind to port {}: {}", port, e);
                return;
            }
        };

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let addr = stream.peer_addr().map_or_else(
                        |_| "unknown".to_string(),
                        |a| a.to_string(),
                    );
                    info!("New incoming peer connection from: {}", addr);

                    let client_channel_clone = client_channel.clone();
                    thread::spawn(move || {
                        if let Err(e) =
                            handle_incoming_peer(stream, client_channel_clone)
                        {
                            error!(
                                "Error handling incoming peer from {}: {}",
                                addr, e
                            );
                        }
                    });
                }
                Err(e) => {
                    warn!("Incoming connection failed: {}", e);
                }
            }
        }
    }
}

fn handle_incoming_peer(
    mut stream: TcpStream,
    client_channel: Sender<ClientOperation>,
) -> io::Result<()> {
    let mut reader = MessageReader::new();

    // Set a read timeout to avoid blocking indefinitely if the peer sends nothing.
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;

    // Read the first message from the peer to identify them
    reader.read_from_socket(&mut stream)?;

    if let Ok(Some(mut message)) = reader.extract_message() {
        message.set_pointer(4); // Skip 4-byte length
        let code = message.read_int8();

        if code == 1 {
            // This must be a PeerInit message
            let username = message.read_string();
            let conn_type_str = message.read_string();
            let token = message.read_raw_bytes(4);

            debug!(
                "Incoming PeerInit from {} with type {}",
                username, conn_type_str
            );

            let peer_info = Peer {
                username,
                connection_type: conn_type_str.parse().unwrap(),
                host: stream.peer_addr()?.ip().to_string(),
                port: 0,
                token: Some(token),
                privileged: 0,
                unknown: 0,
                obfuscated_port: 0,
            };

            let mut default_peer = DefaultPeer::new(peer_info, client_channel);
            default_peer.start_read_write_loops(stream)?;

            if let Some(handle) = default_peer.read_thread.take() {
                handle.join().expect("Peer read thread panicked");
            }
        } else {
            warn!("Unexpected first message code from incoming peer: {}", code);
        }
    } else {
        warn!("Peer connected but sent no initial message.");
    }
    Ok(())
}
