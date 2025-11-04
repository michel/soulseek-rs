use std::net::TcpListener;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::thread;

use crate::client::{ClientContext, ClientOperation};

use crate::message::MessageReader;
use crate::peer::{ConnectionType, DefaultPeer, DownloadPeer, Peer};
use crate::{debug, error, info, trace, DownloadStatus};

pub struct Listen {}

impl Listen {
    pub fn start(
        port: u32,
        client_sender: Sender<ClientOperation>,
        client_context: Arc<RwLock<ClientContext>>,
        own_username: String,
    ) {
        info!("starting listener on port {port}");
        let listener = TcpListener::bind(format!("0.0.0.0:{port}")).unwrap();
        for stream in listener.incoming() {
            let client_sender = client_sender.clone();
            let client_context = client_context.clone();
            let own_username = own_username.clone();
            thread::spawn(move || {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        trace!("Failed to accept connection: {}", e);
                        return;
                    }
                };
                let peer_ip = stream.peer_addr().unwrap().ip().to_string();
                let peer_port = stream.peer_addr().unwrap().port();

                let mut reader = MessageReader::new();

                let mut message = loop {
                    if let Err(e) = reader.read_from_socket(&mut stream) {
                        trace!("Failed to read from socket: {}", e);
                        return;
                    }

                    if let Ok(Some(msg)) = reader.extract_message() {
                        break msg;
                    }
                };

                message.set_pointer(4);
                let message_code = message.read_int8();

                if message_code == 1 {
                    let username = message.read_string();
                    let connection_type = message.read_string();
                    let token = message.read_int32();
                    debug!("[listener:{peer_ip}:{peer_port}] peerInit (0)  username: {username} connection_type: {connection_type} token: {token}");

                    let peer = Peer::new(
                        format!("{}-direct", username.clone()),
                        ConnectionType::P,
                        peer_ip.clone(),
                        peer_port.into(),
                        None,
                        0,
                        0,
                        0,
                    );

                    if connection_type == "P" {
                        debug!("[listener:{peer_ip}:{peer_port}] connection type is P, reader buffer has {} bytes", reader.buffer_len());
                        let default_peer =
                            DefaultPeer::new(peer, client_sender);
                        let default_peer = default_peer
                            .connect_with_socket(stream, Some(reader))
                            .unwrap();

                        drop(default_peer);
                    } else if connection_type == "F" {
                        let maybe_download = {
                            let client_context = client_context.read().unwrap();
                            client_context.download_tokens.get(&token).cloned()
                        };

                        let client_context_clone = client_context.clone();

                        trace!(
                            "[client] DownloadFromPeer token: {} peer: {:?}",
                            token,
                            peer
                        );

                        thread::spawn(move || {
                            let download_peer = DownloadPeer::new(
                                format!("{}-direct", username.clone()),
                                peer.host.clone(),
                                peer.port,
                                token,
                                false,
                                own_username,
                            );

                            match download_peer.download_file(
                                client_context_clone,
                                maybe_download,
                                Some(stream),
                            ) {
                                Ok((download, filename)) => {
                                    download
                                        .sender
                                        .send(DownloadStatus::Completed)
                                        .unwrap();
                                    info!("Successfully downloaded {} bytes to {}", download.size, filename);
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to download file from {}:{} (token: {}) - Error: {}", 
                                        peer.host, peer.port, token, e
                                    );
                                }
                            }
                        });
                    } else {
                        debug!(
                            "[listener:{peer_ip}:{peer_port}] connection type is not P or F"
                        );
                    }
                } else {
                    debug!("[listener:{peer_ip}:{peer_port}] unknown message with code: {message_code}");
                }
            });
        }
    }
}
