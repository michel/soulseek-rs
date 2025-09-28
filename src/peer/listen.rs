use std::sync::mpsc::Sender;
use std::{io, net::TcpListener};

use crate::client::ClientOperation;
use crate::message::MessageReader;

use crate::peer::NewPeer;
use crate::{debug, error, info, trace, warn};

pub struct Listen {}

impl Listen {
    pub fn start(port: u32, client_sender: Sender<ClientOperation>) {
        info!("starting listener on port {port}");
        let listener = TcpListener::bind(format!("0.0.0.0:{port}")).unwrap();
        for stream in listener.incoming() {
            let mut read_stream = stream.unwrap();

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        debug!("Read operation timed out");
                        continue;
                    }
                    Err(e) => {
                        error!("Error reading from server: {}", e);
                        break;
                    }
                }

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
                                );

                                trace!("[listener] Handling peer init message: {:?}",new_peer);
                                client_sender
                                    .send(ClientOperation::NewPeer(new_peer))
                                    .unwrap();
                            }
                            _ => {} // debug!(
                                    //     "[listener] unknown message_code: {}",
                                    //     message_code
                                    // ),
                        }
                    }
                    Err(e) => {
                        warn!("[listener] Error extracting message: {}", e)
                    }
                    Ok(None) => continue,
                }
            }

            // info!("[listener] Connection established!");
        }
    }
}
