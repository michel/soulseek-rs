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

/// The password the client logs in with, and the one it has asked the server
/// to change it to. Only an echo of that request is taken as its
/// confirmation: an echo of anything else would put a password of the
/// sender's choosing into every later login.
#[derive(Debug, Default)]
pub struct LoginPassword {
    pub current: String,
    pub requested: Option<String>,
}

impl LoginPassword {
    #[must_use]
    pub const fn new(current: String) -> Self {
        Self {
            current,
            requested: None,
        }
    }

    pub fn confirm(&mut self, echoed: &str) {
        if self.requested.as_deref() != Some(echoed) {
            return;
        }
        self.current = echoed.to_string();
        self.requested = None;
    }
}

#[derive(Debug, Default)]
pub struct Context {
    pub logged_in: Option<bool>,
    /// The connection dropped. A login waiting on it is never sent again, so
    /// its waiter answers now rather than at the verdict timeout.
    pub connection_lost: bool,
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
