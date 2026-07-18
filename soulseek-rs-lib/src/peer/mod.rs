mod download_peer;
pub mod listen;

// Export actor types
pub use crate::actor::peer_actor::{PeerActor, PeerMessage};
pub use crate::actor::peer_registry::PeerRegistry;

pub use download_peer::DownloadPeer;

use crate::message::Message;
use core::fmt;
use std::{net::TcpStream, str::FromStr};

#[derive(Debug)]
#[allow(dead_code)]
pub struct NewPeer {
    pub username: String,
    pub connection_type: ConnectionType,
    pub token: u32,
    pub tcp_stream: TcpStream,
}
impl NewPeer {
    pub fn new_from_message(
        message: &mut Message,
        tcp_stream: TcpStream,
    ) -> Option<Self> {
        let username = message.read_string();
        let connection_type = message.read_string().parse().ok()?;
        let token = message.read_int32();

        Some(Self {
            username,
            connection_type,
            token,
            tcp_stream,
        })
    }
}

#[derive(Debug, Clone)]
pub enum ConnectionType {
    P,
    F,
    D,
}

#[derive(Debug, Clone)]
pub struct ParseConnectionTypeError;

impl fmt::Display for ParseConnectionTypeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "invalid connection type")
    }
}

impl std::error::Error for ParseConnectionTypeError {}

impl FromStr for ConnectionType {
    type Err = ParseConnectionTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "P" => Ok(Self::P),
            "F" => Ok(Self::F),
            "D" => Ok(Self::D),
            _ => Err(ParseConnectionTypeError),
        }
    }
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::P => "P",
            Self::F => "F",
            Self::D => "D",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Peer {
    pub username: String,
    pub connection_type: ConnectionType,
    pub host: String,
    pub port: u32,
    pub token: Option<u32>,
    pub privileged: Option<u8>,
    pub unknown: Option<u8>,
    pub obfuscated_port: Option<u8>,
}
impl Peer {
    #[allow(clippy::too_many_arguments, dead_code)]
    #[must_use]
    pub const fn new(
        username: String,
        connection_type: ConnectionType,
        host: String,
        port: u32,
        token: Option<u32>,
        privileged: u8,
        unknown: u8,
        obfuscated_port: u8,
    ) -> Self {
        Self {
            username,
            connection_type,
            host,
            port,
            token,
            privileged: Some(privileged),
            unknown: Some(unknown),
            obfuscated_port: Some(obfuscated_port),
        }
    }
    #[allow(dead_code)]
    pub fn new_from_message(message: &mut Message) -> Option<Self> {
        let username = message.read_string();
        // The connection type is an untrusted string; an unknown value must not
        // panic the actor that parses it.
        let connection_type = message.read_string().parse().ok()?;

        let ip = [
            message.read_int8(),
            message.read_int8(),
            message.read_int8(),
            message.read_int8(),
        ];
        let host = format!("{}.{}.{}.{}", ip[3], ip[2], ip[1], ip[0]);

        let (port, token, privileged, unknown, obfuscated_port) = (
            message.read_int32(),
            message.read_int32(),
            message.read_int8(),
            message.read_int8(),
            message.read_int8(),
        );

        Some(Self {
            username,
            connection_type,
            host,
            port,
            token: Some(token),
            privileged: Some(privileged),
            unknown: Some(unknown),
            obfuscated_port: Some(obfuscated_port),
        })
    }
}
#[test]
fn new_from_message_returns_none_on_invalid_connection_type() {
    // username "ab", connection_type "X" (not P/F/D) from an untrusted server.
    let mut data: Vec<u8> = vec![0, 0, 0, 0, 0, 0, 0, 0];
    data.extend([2, 0, 0, 0, 97, 98]); // username = "ab"
    data.extend([1, 0, 0, 0, 88]); // connection_type = "X"
    data.extend([1, 2, 3, 4]); // ip
    data.extend([0, 0, 0, 0]); // port
    data.extend([0, 0, 0, 0]); // token
    data.extend([0, 0, 0]); // privileged, unknown, obfuscated_port
    let mut message = Message::new_with_data(data);
    message.set_pointer(8);

    assert!(Peer::new_from_message(&mut message).is_none());
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

    let peer = Peer::new_from_message(&mut message).unwrap();

    assert_eq!(peer.username, "dp");
    assert!(matches!(peer.connection_type, ConnectionType::P));
    assert_eq!(peer.host, "45.37.231.27");
    assert_eq!(peer.port, 2234);
    assert_eq!(peer.token, Some(1658546));
    assert_eq!(peer.privileged, Some(0));
    assert_eq!(peer.unknown, Some(0));
    assert_eq!(peer.obfuscated_port, Some(0));
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

    println!("code: {}", message.get_message_code_u32());

    let peer = Peer::new_from_message(&mut message).unwrap();

    assert_eq!(peer.username, "grandpag");
    assert!(matches!(peer.connection_type, ConnectionType::P));
    assert_eq!(peer.host, "68.193.128.137");
    assert_eq!(peer.port, 2235);
    assert_eq!(peer.token, Some(4154));
    assert_eq!(peer.privileged, Some(0));
    assert_eq!(peer.unknown, Some(1));
    assert_eq!(peer.obfuscated_port, Some(0));
}
