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
    /// A lock was poisoned by a panic in another thread
    LockPoisoned,
}

impl fmt::Display for SoulseekRs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetworkError(err) => {
                write!(f, "Network error: {err}")
            }
            Self::AuthenticationFailed => {
                write!(f, "Authentication failed")
            }
            Self::ParseError(msg) => write!(f, "Parse error: {msg}"),
            Self::Timeout => write!(f, "Operation timed out"),
            Self::ConnectionClosed => write!(f, "Connection closed"),
            Self::InvalidMessage(msg) => {
                write!(f, "Invalid message: {msg}")
            }
            Self::NotConnected => write!(f, "Not connected to server"),
            Self::CompressionError(msg) => {
                write!(f, "Compression error: {msg}")
            }
            Self::LockPoisoned => {
                write!(f, "Lock poisoned by panicking thread")
            }
        }
    }
}

impl Error for SoulseekRs {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NetworkError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SoulseekRs {
    fn from(err: std::io::Error) -> Self {
        Self::NetworkError(err)
    }
}

impl From<std::num::ParseIntError> for SoulseekRs {
    fn from(err: std::num::ParseIntError) -> Self {
        Self::ParseError(format!("Integer parse error: {err}"))
    }
}

impl From<String> for SoulseekRs {
    fn from(err: String) -> Self {
        Self::CompressionError(err)
    }
}

/// Result type alias for the Soulseek library
pub type Result<T> = std::result::Result<T, SoulseekRs>;
