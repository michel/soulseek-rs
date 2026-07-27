//! JSON-RPC 2.0 framing: what a request, a reply, and an error look like on
//! the wire, independent of the methods they carry.

use crate::output::Exit;
use serde::{Deserialize, Serialize};

/// A request from a client. `id` is absent for a notification, which by
/// JSON-RPC rule gets no reply.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Request {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A reply. Exactly one of `result`/`error` is set; `id` echoes the request,
/// and is absent on a daemon-initiated notification.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Response {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

fn jsonrpc_version() -> String {
    "2.0".to_string()
}

/// A failure, carrying the exit code the same command would have produced
/// locally so a remote run exits exactly as a local one does.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorData {
    /// The `Exit` discriminant this failure maps to.
    pub exit: u8,
}

/// Application errors all share one code; the exit status in `data` is what a
/// caller branches on.
pub const CODE_APPLICATION: i32 = -32000;
pub const CODE_PARSE: i32 = -32700;
pub const CODE_INVALID_REQUEST: i32 = -32600;
pub const CODE_METHOD_NOT_FOUND: i32 = -32601;
pub const CODE_INVALID_PARAMS: i32 = -32602;

impl RpcError {
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// An application failure that should reproduce `exit` on the client.
    #[must_use]
    pub fn application(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            code: CODE_APPLICATION,
            message: message.into(),
            data: Some(ErrorData { exit: exit.code() }),
        }
    }

    /// The exit status this error asks the client to produce, defaulting to a
    /// generic failure when the daemon did not say.
    #[must_use]
    pub fn exit(&self) -> Exit {
        match self.data.as_ref().map(|data| data.exit) {
            Some(0) => Exit::Ok,
            Some(2) => Exit::Usage,
            Some(3) => Exit::Connection,
            Some(4) => Exit::NoResults,
            Some(5) => Exit::Timeout,
            Some(6) => Exit::Transfer,
            Some(7) => Exit::SessionLost,
            _ => Exit::Failure,
        }
    }
}

impl Response {
    #[must_use]
    pub fn result(
        id: Option<serde_json::Value>,
        value: serde_json::Value,
    ) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id,
            method: None,
            params: None,
            result: Some(value),
            error: None,
        }
    }

    /// A failed reply.
    ///
    /// An error always carries an `id` member, explicitly `null` when the
    /// request was too malformed to have one. JSON-RPC requires it, and
    /// without it a client following the "no id means a notification" rule
    /// would drop the error and wait out its timeout for a reply that is
    /// already there.
    #[must_use]
    pub fn failure(id: Option<serde_json::Value>, error: RpcError) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id: Some(id.unwrap_or(serde_json::Value::Null)),
            method: None,
            params: None,
            result: None,
            error: Some(error),
        }
    }

    /// A daemon-initiated event: no id, so no reply is expected.
    #[must_use]
    pub fn notification(method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id: None,
            method: Some(method.to_string()),
            params: Some(params),
            result: None,
            error: None,
        }
    }
}
