//! The values the server actor hands out: where a peer listens, a private
//! message, and the login context.

#[derive(Debug, Clone)]
pub struct PeerAddress {
    host: String,
    port: u16,
}

impl PeerAddress {
    #[must_use]
    pub const fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    #[must_use]
    pub fn get_host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn get_port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Default)]
pub struct Context {
    pub logged_in: Option<bool>,
}

impl Context {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}
#[derive(Debug, Clone)]
pub struct UserMessage {
    id: u32,
    timestamp: u32,
    username: String,
    message: String,
    new_message: bool,
}
impl UserMessage {
    #[must_use]
    pub const fn new(
        id: u32,
        timestamp: u32,
        username: String,
        message: String,
        new_message: bool,
    ) -> Self {
        Self {
            id,
            timestamp,
            username,
            message,
            new_message,
        }
    }
    /// The server-assigned id of this message (used to acknowledge it).
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Unix timestamp the server recorded for this message.
    #[must_use]
    pub const fn timestamp(&self) -> u32 {
        self.timestamp
    }

    /// The username of the sender.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The message body.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the server flagged this as freshly delivered (as opposed to a
    /// message replayed because it was queued while the recipient was offline).
    #[must_use]
    pub const fn is_new(&self) -> bool {
        self.new_message
    }
}
