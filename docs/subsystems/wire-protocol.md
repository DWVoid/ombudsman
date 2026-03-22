# Wire Protocol

**Source:** `ombudsman-core/src/protocol.rs`

The `ombudsman-core` crate defines the shared wire protocol used by all clients (`ombudsman-cli`, `ombudsman-gui`) to communicate with `ombudsman-server`.

---

## Table of Contents

1. [Transport](#1-transport)
2. [Serialization](#2-serialization)
3. [ClientMsg — Client → Server](#3-clientmsg--client--server)
4. [ServerMsg — Server → Client](#4-servermsg--server--client)
5. [SessionInfo](#5-sessioninfo)
6. [Codec Functions](#6-codec-functions)

---

## 1. Transport

All protocol messages are carried over **WebSocket binary frames**.  The server listens at `ws://<host>:<port>/ws` (default `ws://127.0.0.1:7878/ws`).

Text frames, ping, and pong frames are ignored by both sides.

---

## 2. Serialization

Messages are serialized with **MessagePack** using `rmp-serde` (`to_vec_named` / `from_slice`).  The `named` variant preserves field names, enabling forward compatibility when new optional fields are added.

Both `ClientMsg` and `ServerMsg` use `#[serde(tag = "type", rename_all = "snake_case")]`, so every message carries a `"type"` discriminator field.

---

## 3. ClientMsg — Client → Server

```rust
pub enum ClientMsg {
    Chat {
        session_key: String,
        content: String,
        media: Vec<String>,   // optional file/image paths
    },
    Command {
        session_key: String,
        command: String,      // "/new", "/stop", "/status", "/sessions", "/help"
    },
    SessionList,
    Stop { session_key: String },
}
```

| Variant | Description |
|---|---|
| `Chat` | Send a user message to the agent for the given session |
| `Command` | Send a slash command (same as typing `/new` etc. in the CLI) |
| `SessionList` | Ask the server for a list of all sessions |
| `Stop` | Request the server to abort the current agent task for a session |

---

## 4. ServerMsg — Server → Client

```rust
pub enum ServerMsg {
    Progress { content: String, is_tool_hint: bool },
    Response { content: String, media: Vec<String> },
    Error { message: String },
    Ack { session_key: String },
    SessionList { sessions: Vec<SessionInfo> },
    Stopped { session_key: String },
    StatusResponse { content: String },
}
```

| Variant | When sent |
|---|---|
| `Progress` | Intermediate update while the agent is thinking or running a tool. `is_tool_hint = true` signals a tool-call status line. |
| `Response` | Final agent reply; closes the current "turn". `media` contains output file paths when the agent produced files. |
| `Error` | A server-side error occurred (e.g. provider failure, no API key). |
| `Ack` | The server accepted a `Chat` or `Command` request and has started processing. |
| `SessionList` | Response to `ClientMsg::SessionList`. |
| `Stopped` | The running task was cancelled (response to `Stop`) or there was nothing to cancel. |
| `StatusResponse` | Response to a `/status` command — server configuration summary. |

---

## 5. SessionInfo

Returned inside `ServerMsg::SessionList`:

```rust
pub struct SessionInfo {
    pub key:           String,   // e.g. "cli:default"
    pub message_count: usize,
    pub updated_at:    String,   // RFC-3339 timestamp
}
```

---

## 6. Codec Functions

```rust
pub fn encode_server_msg(msg: &ServerMsg) -> anyhow::Result<Vec<u8>>
pub fn decode_client_msg(bytes: &[u8]) -> anyhow::Result<ClientMsg>
pub fn encode_client_msg(msg: &ClientMsg) -> anyhow::Result<Vec<u8>>
pub fn decode_server_msg(bytes: &[u8]) -> anyhow::Result<ServerMsg>
```

All four functions are used by both the server and clients.  Encoding errors are wrapped as `anyhow::Error`; decoding errors are also wrapped and cause the receiving side to log a warning and discard the frame.
