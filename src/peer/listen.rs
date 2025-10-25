use std::sync::mpsc::Sender;
use std::{io, net::TcpListener};

use crate::client::ClientOperation;
use crate::message::MessageReader;

use crate::{debug, error, info, trace, warn, FileSearchResult};

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
                    Ok(()) => {}
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

                // Extract all available messages from buffer
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
                                    // let token = message.read_string();
                                    // debug!("[listener] received Pierce Firewall, token: {}", token);
                                }
                                1 => {
                                    // handle handover to default_peer with socket in the future
                                    // let tcp_stream =
                                    //     read_stream.try_clone().unwrap();
                                    //
                                    // let username = message.read_string();
                                    // let connection_type =
                                    //     message.read_string().parse().unwrap();
                                    // let token = message.read_int32();
                                }
                                9 => {
                                    trace!("[listener] Handling file search response: 9",);
                                    message.set_pointer(8);
                                    let file_search =
                                        match FileSearchResult::new_from_message(
                                            &mut message,
                                        ) {
                                            Ok(result) => result,
                                            Err(e) => {
                                                trace!("[listener] malformed filesearch_result: {:?}, message: {:?}", e, message);
                                                return;
                                            }
                                        };
                                    client_sender
                                        .send(ClientOperation::SearchResult(
                                            file_search,
                                        ))
                                        .unwrap();
                                    break;
                                }
                                _ => {
                                    debug!(
                                        "[listener] unknown message_code: {}",
                                        message_code
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
            }

            // info!("[listener] Connection established!");
        }
    }
}
