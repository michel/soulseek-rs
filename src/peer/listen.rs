use std::{io, net::TcpListener};

use crate::message::MessageReader;

pub struct Listen {}

impl Listen {
    pub fn new(port: u32) {
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
                    Ok(Some(message)) => {
                        println!("Received message: {:?}", message.get_message_code_u32());
                        println!("{:?}", message.get_data());
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
