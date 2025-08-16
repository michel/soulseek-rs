mod default_peer;
mod distributed_peer;
mod download_peer;
pub mod listen;

// Re-export commonly used items
pub use default_peer::{DefaultPeer, PeerOperation};
pub use distributed_peer::DistributedPeer;
pub use download_peer::DownloadPeer;

// ADD THIS ENUM
#[allow(dead_code)]
pub enum PeerConnection {
    Default(DefaultPeer),
    Distributed(DistributedPeer),
}

impl PeerConnection {
    pub fn transfer_request(
        &self,
        download: crate::types::Download,
    ) -> Result<(), std::io::Error> {
        match self {
            PeerConnection::Default(peer) => peer.transfer_request(download),
            PeerConnection::Distributed(_) => {
                // Distributed peers don't handle transfer requests
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Distributed peers don't support transfer requests",
                ))
            }
        }
    }
}

use crate::{info, message::Message};
use core::fmt;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum ConnectionType {
    P,
    F,
    D,
}

// Define the error type
#[derive(Debug, Clone)]
pub struct ParseConnectionTypeError;

impl fmt::Display for ParseConnectionTypeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "invalid connection type")
    }
}

impl std::error::Error for ParseConnectionTypeError {}

// Implement FromStr for ConnectionType
impl FromStr for ConnectionType {
    type Err = ParseConnectionTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "P" => Ok(ConnectionType::P),
            "F" => Ok(ConnectionType::F),
            "D" => Ok(ConnectionType::D),
            _ => Err(ParseConnectionTypeError),
        }
    }
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            ConnectionType::P => "P",
            ConnectionType::F => "F",
            ConnectionType::D => "D",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Peer {
    pub username: String,
    pub connection_type: ConnectionType,
    pub host: String,
    pub port: u32,
    pub token: Option<Vec<u8>>,
    pub privileged: u8,
    pub unknown: u8,
    pub obfuscated_port: u8,
}
impl Peer {
    #[allow(clippy::too_many_arguments, dead_code)]
    pub fn new(
        username: String,
        connection_type: ConnectionType,
        host: String,
        port: u32,
        token: Vec<u8>,
        privileged: u8,
        unknown: u8,
        obfuscated_port: u8,
    ) -> Self {
        Self {
            username,
            connection_type,
            host,
            port,
            token: Some(token),
            privileged,
            unknown,
            obfuscated_port,
        }
    }
    #[allow(dead_code)]
    pub fn new_from_message(message: &mut Message) -> Self {
        let username = message.read_string();
        let raw_connection_type = message.read_string();
        let connection_type = raw_connection_type.parse().unwrap();

        let mut ip: Vec<i32> = vec![];
        for _ in 0..4 {
            ip.push(message.read_int8().into());
        }
        let host: String = format!(
            "{}.{}.{}.{}",
            ip[3].abs(),
            ip[2].abs(),
            ip[1].abs(),
            ip[0].abs()
        );

        let (port, token, privileged, unknown, obfuscated_port) = (
            message.read_int32(),
            message.read_raw_bytes(4),
            message.read_int8(),
            message.read_int8(),
            message.read_int8(),
        );

        if raw_connection_type == "F" {
            info!(
                "ConnectToPeer: {} {} {} {} {:?}",
                username, connection_type, host, port, token
            );
        }

        Self {
            username,
            connection_type,
            host,
            port,
            token: Some(token),
            privileged,
            unknown,
            obfuscated_port,
        }
    }
}
#[test]
fn test_new_from_message() {
    let data: Vec<u8> = [
        36, 0, 0, 0, 18, 0, 0, 0, 2, 0, 0, 0, 100, 112, 1, 0, 0, 0, 80, 27,
        231, 37, 45, 186, 8, 0, 0, 178, 78, 25, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]
    .to_vec();
    let mut message = Message::new_with_data(data);
    message.set_pointer(8);

    let peer = Peer::new_from_message(&mut message);

    assert_eq!(peer.username, "dp");
    assert!(matches!(peer.connection_type, ConnectionType::P));
    assert_eq!(peer.host, "45.37.231.27");
    assert_eq!(peer.port, 2234);
    assert_eq!(peer.token, Some([178, 78, 25, 0].to_vec()));
    assert_eq!(peer.privileged, 0);
    assert_eq!(peer.unknown, 0);
    assert_eq!(peer.obfuscated_port, 0);
}

#[test]
fn test_new_from_message2() {
    let data: Vec<u8> = [
        42, 0, 0, 0, 18, 0, 0, 0, 8, 0, 0, 0, 103, 114, 97, 110, 100, 112, 97,
        103, 1, 0, 0, 0, 80, 137, 128, 193, 68, 187, 8, 0, 0, 58, 16, 0, 0, 0,
        1, 0, 0, 0, 188, 8, 0, 0,
    ]
    .to_vec();
    let mut message = Message::new_with_data(data);
    message.set_pointer(8);

    let peer = Peer::new_from_message(&mut message);

    assert_eq!(peer.username, "grandpag");
    assert!(matches!(peer.connection_type, ConnectionType::P));
    assert_eq!(peer.host, "68.193.128.137");
    assert_eq!(peer.port, 2235);
    assert_eq!(peer.token, Some([58, 16, 0, 0].to_vec()));
    assert_eq!(peer.privileged, 0);
    assert_eq!(peer.unknown, 1);
    assert_eq!(peer.obfuscated_port, 0);
}
