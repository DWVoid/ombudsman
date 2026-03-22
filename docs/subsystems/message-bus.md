# Message Bus

**Source:** `src/bus/`

The message bus decouples transport channels (CLI, and future integrations such as Telegram or Discord) from the agent processing core. The agent loop only knows about the bus; it never talks directly to a terminal or network socket.

---

## Table of Contents

1. [Design Rationale](#1-design-rationale)
2. [MessageBus](#2-messagebus)
3. [Event Types](#3-event-types)
4. [Session Key Derivation](#4-session-key-derivation)

---

## 1. Design Rationale

Using an async MPSC bus:

- **Transport independence** — the CLI, a future Telegram bot, or any other channel only needs to `publish_inbound` and `consume_outbound`. The agent loop is identical regardless of the transport.
- **Backpressure** — the bounded channel capacity (configured at construction time) naturally limits the number of unprocessed messages in flight.
- **Testability** — tests can drive the agent loop by publishing to the inbound channel and asserting on the outbound channel, without any real terminal or network.

---

## 2. MessageBus

**File:** `src/bus/mod.rs`

### Struct

```rust
pub struct MessageBus {
    inbound_tx:  mpsc::Sender<InboundMessage>,
    inbound_rx:  tokio::sync::Mutex<mpsc::Receiver<InboundMessage>>,
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: tokio::sync::Mutex<mpsc::Receiver<OutboundMessage>>,
}
```

The receivers are wrapped in `tokio::sync::Mutex` so they can be held in an `Arc<MessageBus>` and polled from async contexts without requiring `&mut self`.

### Construction

```rust
MessageBus::new(capacity: usize) -> Self
```

Creates one inbound and one outbound `tokio::sync::mpsc` channel pair, each with the given buffer capacity.

### Methods

| Method | Direction | Description |
|---|---|---|
| `publish_inbound(msg)` | Channel → Agent | Send a message to the agent |
| `consume_inbound()` | Channel → Agent | Await the next inbound message |
| `publish_outbound(msg)` | Agent → Channel | Send a reply from the agent |
| `consume_outbound()` | Agent → Channel | Await the next outbound message |
| `inbound_sender()` | — | Clone the inbound `Sender` for multi-producer use |
| `outbound_sender()` | — | Clone the outbound `Sender` for multi-producer use |

### Typical Usage Pattern

```
CLI thread                         Agent thread
──────────────────────────────     ─────────────────────────────
bus.publish_inbound(msg)    ──→    msg = bus.consume_inbound()
                                   … process …
reply = bus.consume_outbound() ←── bus.publish_outbound(reply)
```

---

## 3. Event Types

**File:** `src/bus/events.rs`

### InboundMessage

Represents a message arriving from an external channel (e.g., the user's typed input).

```rust
pub struct InboundMessage {
    pub channel:              String,
    pub sender_id:            String,
    pub chat_id:              String,
    pub content:              String,
    pub timestamp:            DateTime<Local>,
    pub media:                Vec<String>,
    pub metadata:             HashMap<String, serde_json::Value>,
    pub session_key_override: Option<String>,
}
```

| Field | Description |
|---|---|
| `channel` | Transport name, e.g. `"cli"` |
| `sender_id` | Identity of the sender within the channel |
| `chat_id` | The conversation thread identifier within the channel |
| `content` | The user's text message |
| `timestamp` | Local time when the message was received |
| `media` | List of media attachment paths or URLs (currently unused by CLI) |
| `metadata` | Arbitrary key-value pairs for channel-specific extensions |
| `session_key_override` | If set, overrides the default session key derivation |

### OutboundMessage

Represents a reply produced by the agent.

```rust
pub struct OutboundMessage {
    pub channel:  String,
    pub chat_id:  String,
    pub content:  String,
    pub reply_to: Option<String>,
    pub media:    Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}
```

| Field | Description |
|---|---|
| `channel` | Destination transport |
| `chat_id` | Conversation thread to reply to |
| `content` | The agent's reply text |
| `reply_to` | Optional message ID to reply to (used by chat platforms) |
| `media` | Outbound media attachments |
| `metadata` | Channel-specific extension data |

---

## 4. Session Key Derivation

The session key identifies a unique conversation thread and is used to load and persist the correct session history file.

Default derivation:

```
session_key = "{channel}:{chat_id}"
```

For example, the CLI uses `channel = "cli"` and `chat_id = "default"`, producing the session key `cli:default`.

If `session_key_override` is set in the `InboundMessage`, that value is used directly. This allows an external orchestrator to route messages from different channels into a single shared session.
