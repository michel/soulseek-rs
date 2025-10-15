use std::sync::mpsc::Sender;
use std::{io, net::TcpListener};

use crate::client::ClientOperation;
use crate::message::{MessageReader, MessageType};

use crate::peer::NewPeer;
use crate::{debug, error, info, trace, warn};

pub struct Listen {}

impl Listen {
    pub fn start(port: u32, client_sender: Sender<ClientOperation>) {
        info!("[listener] starting listener on port {port}");
        let listener = TcpListener::bind(format!("0.0.0.0:{port}")).unwrap();
        for stream in listener.incoming() {
            let mut read_stream = stream.unwrap();

            let mut buffered_reader = MessageReader::new();
            match buffered_reader.read_from_socket(&mut read_stream) {
                Ok(_) => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    debug!("[listener] Read operation timed out");
                    continue;
                }
                Err(e) => {
                    error!("[listener] Error reading from server: {}", e);
                    break;
                }
            }

            loop {
                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => {
                        message.set_pointer(4);
                        let message_code = message.read_int8();

                        trace!(
                            "[listener] Received message with code: {}",
                            message_code
                        );

                        match message_code {
                            0 => {
                                let token = message.read_string();
                                debug!("[listener] received Pierce Firewall, token: {}", token);
                            }
                            1 => {
                                let tcp_stream =
                                    read_stream.try_clone().unwrap();
                                let new_peer = NewPeer::new_from_message(
                                    &mut message,
                                    tcp_stream,
                                    buffered_reader.clone(),
                                );

                                trace!("[listener] Handling peer init message: {:?}",new_peer);
                                client_sender
                                    .send(ClientOperation::NewPeer(new_peer))
                                    .unwrap();
                                break;
                            }

                            _ => {
                                debug!(
                                    "[listener] unhandled code: {} - {}",
                                    message_code,
                                    message
                                        .get_message_name(
                                            MessageType::Peer,
                                            message_code as u32
                                        )
                                        .unwrap_or("unknown")
                                )
                            }
                        }
                    }
                    Err(e) => {
                        warn!("[listener] Error extracting message: {}", e);
                        break;
                    }
                    Ok(None) => break,
                }
            }
            trace!("[listener] done with stream");
        }
    }
}
