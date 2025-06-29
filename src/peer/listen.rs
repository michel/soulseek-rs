use std::sync::mpsc::Sender;
use std::{io, net::TcpListener};

use crate::message::MessageReader;
use crate::server::ServerOperation;

pub struct Listen {}

impl Listen {
    pub fn start(port: u32, _server_channel: Sender<ServerOperation>) {
        println!("starting listener on port {port}");
        let listener = TcpListener::bind(format!("0.0.0.0:{port}")).unwrap();
        for stream in listener.incoming() {
            let mut read_stream = stream.unwrap();

            let mut buffered_reader = MessageReader::new();
            loop {
                match buffered_reader.read_from_socket(&mut read_stream) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        println!("Read operation timed out");
                        continue;
                    }
                    Err(e) => {
                        eprintln!("Error reading from server: {}", e);
                        break;
                    }
                }

                match buffered_reader.extract_message() {
                    Ok(Some(mut message)) => {
                        println!("Received message: {:?}", message.get_message_code());
                        println!("{:?}", message.get_data());
                        message.print_hex();

                        if message.get_message_code() == 1 {
                            let size = message.read_int32();
                            message.set_pointer(8);
                            let typex = message.read_string();

                            // let user = message.read_string();
                            // // let connection_type: ConnectionType =
                            // //     message.read_string().parse().unwrap();
                            // // let token = message.read_int32();
                            //
                            println!("type: {:?}", typex);
                            println!("user: {:?}", size);

                            // server_channel
                            //     .send(ServerOperation::ConnectToPeer(peer))
                            //     .unwrap();
                        }
                    }
                    Err(e) => {
                        println!("Error extracting message: {}", e)
                    }
                    Ok(None) => continue,
                }
            }

            println!("Connection established!");
        }
    }
}
