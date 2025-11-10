use std::{error::Error, fmt};

/// Custom error type for the Soulseek download library
#[derive(Debug)]
pub enum SoulseekRs {
    NetworkError(std::io::Error),
    /// Authentication failed during login
    AuthenticationFailed,
    /// Error parsing messages or data
    ParseError(String),
    /// Operation timed out
    Timeout,
    /// Connection was closed unexpectedly
    ConnectionClosed,
    /// Invalid message format or content
    InvalidMessage(String),
    /// Server not connected
    NotConnected,
    /// Compression/decompression error
    CompressionError(String),
}

impl fmt::Display for SoulseekRs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SoulseekRs::NetworkError(err) => {
                write!(f, "Network error: {}", err)
            }
            SoulseekRs::AuthenticationFailed => {
                write!(f, "Authentication failed")
            }
            SoulseekRs::ParseError(msg) => write!(f, "Parse error: {}", msg),
            SoulseekRs::Timeout => write!(f, "Operation timed out"),
            SoulseekRs::ConnectionClosed => write!(f, "Connection closed"),
            SoulseekRs::InvalidMessage(msg) => {
                write!(f, "Invalid message: {}", msg)
            }
            SoulseekRs::NotConnected => write!(f, "Not connected to server"),
            SoulseekRs::CompressionError(msg) => {
                write!(f, "Compression error: {}", msg)
            }
        }
    }
}

impl Error for SoulseekRs {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            SoulseekRs::NetworkError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SoulseekRs {
    fn from(err: std::io::Error) -> Self {
        SoulseekRs::NetworkError(err)
    }
}

impl From<std::num::ParseIntError> for SoulseekRs {
    fn from(err: std::num::ParseIntError) -> Self {
        SoulseekRs::ParseError(format!("Integer parse error: {}", err))
    }
}

impl From<String> for SoulseekRs {
    fn from(err: String) -> Self {
        SoulseekRs::CompressionError(err)
    }
}

/// Result type alias for the Soulseek library
pub type Result<T> = std::result::Result<T, SoulseekRs>;
