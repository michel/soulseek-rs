use core::fmt;
use std::str::FromStr;

use crate::message::Message;

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

#[derive(Debug, Clone)]
pub struct Peer {
    pub username: String,
    pub connection_type: ConnectionType,
    pub host: String,
    pub port: i32,
    pub token: Option<String>,
    pub privileged: i8,
    pub unknown: i8,
    pub obfuscated_port: i8,
}
impl Peer {
    pub fn new(
        username: String,
        connection_type: ConnectionType,
        host: String,
        port: i32,
        token: String,
        privileged: i8,
        unknown: i8,
        obfuscated_port: i8,
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
    pub fn new_from_message(message: &mut Message) -> Self {
        let username = message.read_string();
        let connection_type: ConnectionType = message.read_string().parse().unwrap();

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
            message.read_raw_hex_str(4),
            message.read_int8(),
            message.read_int8(),
            message.read_int8(),
        );

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
        36, 0, 0, 0, 18, 0, 0, 0, 2, 0, 0, 0, 100, 112, 1, 0, 0, 0, 80, 27, 231, 37, 45, 186, 8, 0,
        0, 178, 78, 25, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]
    .to_vec();
    let mut message = Message::new_with_data(data);
    message.set_pointer(8);

    let peer = Peer::new_from_message(&mut message);

    assert_eq!(peer.username, "dp");
    assert!(matches!(peer.connection_type, ConnectionType::P));
    assert_eq!(peer.host, "45.37.25.27");
    assert_eq!(peer.port, 2234);
    assert_eq!(peer.token, Some("b24e1900".to_string()));
    assert_eq!(peer.privileged, 0);
    assert_eq!(peer.unknown, 0);
    assert_eq!(peer.obfuscated_port, 0);
}

#[test]
fn test_new_from_message2() {
    let data: Vec<u8> = [
        42, 0, 0, 0, 18, 0, 0, 0, 8, 0, 0, 0, 103, 114, 97, 110, 100, 112, 97, 103, 1, 0, 0, 0, 80,
        137, 128, 193, 68, 187, 8, 0, 0, 58, 16, 0, 0, 0, 1, 0, 0, 0, 188, 8, 0, 0,
    ]
    .to_vec();
    let mut message = Message::new_with_data(data);
    message.set_pointer(8);

    let peer = Peer::new_from_message(&mut message);

    assert_eq!(peer.username, "grandpag");
    assert!(matches!(peer.connection_type, ConnectionType::P));
    assert_eq!(peer.host, "68.63.128.119");
    assert_eq!(peer.port, 2235);
    assert_eq!(peer.token, Some("3a100000".to_string()));
    assert_eq!(peer.privileged, 0);
    assert_eq!(peer.unknown, 1);
    assert_eq!(peer.obfuscated_port, 0);
}
