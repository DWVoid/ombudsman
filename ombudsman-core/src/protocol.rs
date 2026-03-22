//! Wire protocol between ombudsman-server and clients.
//!
//! Messages are serialized with MessagePack (rmp-serde) over WebSocket binary frames.

use serde::{Deserialize, Serialize};

/// Brief information about a session returned by `ServerMsg::SessionList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session key (e.g. "cli:default").
    pub key: String,
    /// Number of messages in the session.
    pub message_count: usize,
    /// RFC-3339 timestamp of last update.
    pub updated_at: String,
}

/// Messages sent from a client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Send a chat message.
    Chat {
        /// Session identifier (e.g. "cli:default").
        session_key: String,
        /// Text content of the message.
        content: String,
        /// Optional file/image paths to attach.
        #[serde(default)]
        media: Vec<String>,
    },
    /// Send a slash command (/new, /stop, /status, /help).
    Command {
        session_key: String,
        command: String,
    },
    /// Request the list of all sessions on the server.
    SessionList,
    /// Request to stop the currently running agent task for a session.
    Stop { session_key: String },
}

/// Messages sent from the server to a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Intermediate progress update (thinking or tool hint).
    Progress {
        content: String,
        is_tool_hint: bool,
    },
    /// Final response from the agent.
    Response {
        content: String,
        #[serde(default)]
        media: Vec<String>,
    },
    /// Server-side error.
    Error { message: String },
    /// Acknowledgement: the server accepted the request.
    Ack { session_key: String },
    /// Response to a `ClientMsg::SessionList` request.
    SessionList { sessions: Vec<SessionInfo> },
    /// The running task was stopped (or there was nothing to stop).
    Stopped { session_key: String },
    /// Status information (response to `/status` command).
    StatusResponse { content: String },
}

/// Encode a `ServerMsg` to MessagePack bytes.
pub fn encode_server_msg(msg: &ServerMsg) -> anyhow::Result<Vec<u8>> {
    rmp_serde::to_vec_named(msg).map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))
}

/// Decode a `ClientMsg` from MessagePack bytes.
pub fn decode_client_msg(bytes: &[u8]) -> anyhow::Result<ClientMsg> {
    rmp_serde::from_slice(bytes).map_err(|e| anyhow::anyhow!("msgpack decode error: {}", e))
}

/// Encode a `ClientMsg` to MessagePack bytes.
pub fn encode_client_msg(msg: &ClientMsg) -> anyhow::Result<Vec<u8>> {
    rmp_serde::to_vec_named(msg).map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))
}

/// Decode a `ServerMsg` from MessagePack bytes.
pub fn decode_server_msg(bytes: &[u8]) -> anyhow::Result<ServerMsg> {
    rmp_serde::from_slice(bytes).map_err(|e| anyhow::anyhow!("msgpack decode error: {}", e))
}