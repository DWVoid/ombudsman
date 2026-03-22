# Session Management

**Source:** `src/session/mod.rs`

The session subsystem persists conversation history across requests, enabling the agent to maintain context within a chat thread.

---

## Table of Contents

1. [Session](#1-session)
2. [SessionManager](#2-sessionmanager)
3. [JSONL Persistence Format](#3-jsonl-persistence-format)
4. [Consolidation Boundary Tracking](#4-consolidation-boundary-tracking)
5. [Legal History Slicing](#5-legal-history-slicing)

---

## 1. Session

A `Session` holds the in-memory state of one conversation.

```rust
pub struct Session {
    pub messages:          Vec<HashMap<String, Value>>,
    pub metadata:          HashMap<String, Value>,
    pub last_consolidated: usize,
    pub created_at:        DateTime<Local>,
    pub updated_at:        DateTime<Local>,
}
```

| Field | Description |
|---|---|
| `messages` | Ordered list of chat messages (system, user, assistant, tool) |
| `metadata` | Arbitrary key-value data attached to the session |
| `last_consolidated` | Index into `messages` marking the end of the last memory consolidation |
| `created_at` | Timestamp when the session was first created |
| `updated_at` | Timestamp of the most recent update |

### Message Format

Each message is a `HashMap<String, Value>` following the OpenAI chat-completions format:

```json
{ "role": "user",      "content": "Hello!" }
{ "role": "assistant", "content": "Hi there!", "tool_calls": […] }
{ "role": "tool",      "content": "…result…", "tool_call_id": "call_abc" }
```

---

## 2. SessionManager

Manages the lifecycle of all sessions: loading, saving, and deleting.

```rust
pub struct SessionManager {
    sessions_dir: PathBuf,
    sessions:     HashMap<String, Session>,
}
```

### Construction

```rust
SessionManager::new(workspace: &Path) -> anyhow::Result<Self>
```

Creates (or confirms the existence of) `workspace/sessions/`. No sessions are pre-loaded; each session is loaded lazily on first access.

### Core Methods

| Method | Description |
|---|---|
| `get_or_create(key)` | Returns the session for `key`, loading from disk if needed or creating a new one |
| `save(key, session)` | Persists the session to `workspace/sessions/<key>.jsonl` |
| `delete(key)` | Deletes the session file and removes it from the in-memory cache |
| `get_history(key, offset, limit)` | Returns a legal slice of messages (see §5) |

### Session Key to File Path

The session key is sanitised to produce a valid filename:
- All characters except alphanumerics, hyphens, and underscores are replaced with `_`.
- The file is stored as `sessions/<sanitised_key>.jsonl`.

For example, the CLI's `cli:default` session key maps to `sessions/cli_default.jsonl`.

---

## 3. JSONL Persistence Format

Sessions are stored as **newline-delimited JSON (JSONL)**: one JSON object per line.

```
{"role":"user","content":"What is Rust?"}
{"role":"assistant","content":"Rust is a systems programming language…"}
{"role":"assistant","content":null,"tool_calls":[{"id":"call_a1b2","type":"function","function":{"name":"web_search","arguments":"{\"query\":\"Rust programming language\"}"}}]}
{"role":"tool","content":"…search results…","tool_call_id":"call_a1b2"}
```

**Why JSONL?**

- Appending a new message is an O(1) write: just append one line.
- Loading the history requires only a sequential read; no JSON array parsing overhead on partial loads.
- The file is human-readable and can be inspected or edited with standard tools.
- Tool-call IDs appear inline, making boundary detection straightforward.

---

## 4. Consolidation Boundary Tracking

The `last_consolidated` field is an index into `messages` that indicates how many messages have already been incorporated into `MEMORY.md`.  It serves two purposes:

1. **Memory consolidation** — `MemoryConsolidator` only passes messages after `last_consolidated` to the LLM, avoiding re-summarising already-consolidated content.
2. **History trim safety** — The agent may trim old messages to fit within the context window, but it must never trim below the legal start (see §5).

After a successful consolidation run, `last_consolidated` is advanced to the current end of `messages`.

---

## 5. Legal History Slicing

When the full session history would exceed the LLM's context window, the agent trims old messages.  However, trimming is constrained by **legal start detection** to avoid breaking the OpenAI message format.

### Rules for Legal Start

A position in the message array is a legal start if:
1. The message at that position is a `user` message.
2. Every `tool_call_id` referenced by subsequent `tool` messages already has a matching `assistant` message with a `tool_calls` entry in the remaining history.

In other words: the agent never trims an `assistant` message while leaving its corresponding `tool` result in the history, and vice versa.

### Offset / Limit Parameters

`get_history(key, offset, limit)`:
- `offset` — number of messages to skip from the beginning (after applying legal-start detection).
- `limit` — maximum number of messages to return.
- Both parameters are optional; omitting them returns the full history.

This slicing is used by the agent loop to stay within the configured `context_window_tokens` budget.
