//! Wire protocol between ombudsman-server and clients.
//!
//! Messages are serialized with MessagePack (rmp-serde) over WebSocket binary frames.

use serde::{Deserialize, Serialize};

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
