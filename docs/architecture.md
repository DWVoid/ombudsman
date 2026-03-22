# Architecture Overview

## Purpose

Ombudsman is a personal AI assistant that accepts natural-language requests, decides which tools to use, executes them, and returns a response — all driven by an LLM backend.  It is implemented as a **Cargo workspace** of four Rust crates with a client/server architecture: an always-on WebSocket server that runs the agent, and thin client frontends (CLI and GUI) that connect to it.

---

## Workspace Crates

| Crate | Binary | Role |
|---|---|---|
| `ombudsman-server` | `ombudsman-server` | WebSocket server; hosts the agent loop, all providers, session and memory management |
| `ombudsman-cli` | `ombudsman-cli` | Interactive terminal client; connects to the server via WebSocket |
| `ombudsman-gui` | `ombudsman-gui` | Desktop GUI client built with iced; connects to the server via WebSocket |
| `ombudsman-core` | _(library)_ | Shared wire protocol types (`ClientMsg`, `ServerMsg`) and MessagePack codec |

---

## High-Level Component Map

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       ombudsman-server process                           │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │          axum WebSocket Server  (ombudsman-server/src/ws.rs)     │   │
│  │   ws://host:7878/ws                                              │   │
│  │   One AgentLoop (+ SubagentManager) per WS connection           │   │
│  └──────────────────────────┬────────────────────────────────────────┘  │
│                             │ InboundMessage / OutboundMessage           │
│                             ▼                                            │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                    Message Bus                                    │   │
│  │              (ombudsman-server/src/bus/)                         │   │
│  └──────────────────────────┬─────────────────────────────────────┘    │
│                             ▼                                            │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                      Agent Loop                                   │   │
│  │         (ombudsman-server/src/agent/loop_runner.rs)              │   │
│  │                                                                   │   │
│  │  1. build system prompt (ContextBuilder)                         │   │
│  │  2. load session history (SessionManager)                        │   │
│  │  3. call LLM  ←→  tool execution loop (≤40 iterations)          │   │
│  │  4. consolidate memory (MemoryConsolidator)                      │   │
│  │  5. publish reply  +  spawn/message via SubagentManager          │   │
│  └────┬─────────────────────────────────┬────────────────────────────┘  │
│       │                                 │ SpawnTool / MessageTool        │
│  ┌────┴────────────┐  ┌──────────────┐  ▼                               │
│  │ Context Builder │  │  Session Mgr │  ┌───────────────────────────┐   │
│  │  AGENTS.md      │  │  JSONL files │  │   SubagentManager         │   │
│  │  SOUL.md        │  │  per session │  │  tokio::spawn tasks       │   │
│  │  USER.md        │  └──────────────┘  └───────────────────────────┘   │
│  └─────────────────┘                                                     │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                     Tool Registry                                 │   │
│  │  read_file  write_file  edit_file  list_dir  exec                │   │
│  │  web_search  web_fetch  spawn  message                           │   │
│  │  mcp_{server}_{tool}  (from MCP servers, added lazily)           │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  LLM Providers  (ombudsman-server/src/providers/)                │   │
│  │  OpenAIProvider · AnthropicProvider · AzureOpenAIProvider        │   │
│  │  Provider Registry (15 entries incl. gateways)                   │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  Configuration  (ombudsman-server/src/config/)                   │   │
│  │  ~/.ombudsman/config.json                                         │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────┘

          ▲ WebSocket (MessagePack) ▲
          │  ombudsman-core proto   │
   ┌──────┴─────┐         ┌────────┴────────┐
   │ombudsman-  │         │  ombudsman-gui  │
   │    cli     │         │  (iced MVVM)    │
   │  terminal  │         │  desktop window │
   └────────────┘         └─────────────────┘
```

---

## Data-Flow: Handling One User Message

```
Client (CLI or GUI)
   • user types message
   • encode ClientMsg::Chat as MessagePack
   • send over WebSocket binary frame
          │
          ▼
ombudsman-server ws.rs  handle_session()
   • decode ClientMsg
   • bus.publish_inbound(InboundMessage)
          │
          ▼
AgentLoop.process_direct()
   │
   ├─ 1. ContextBuilder.build_system_prompt()
   │       reads workspace/AGENTS.md, SOUL.md, USER.md, TOOLS.md
   │       reads memory/MEMORY.md
   │       reads workspace/skills/**/SKILL.md
   │
   ├─ 2. SessionManager.get_history(session_key)
   │       loads JSONL from workspace/sessions/<session_key>.jsonl
   │
   ├─ 3. Build messages[] = [system] + [history] + [new user msg]
   │
   ├─ 4. Loop (up to max_iterations)
   │   │
   │   │   [on first iteration: lazily connect to MCP servers]
   │   │
   │   ├─ LLMProvider.chat(model, messages, tools, settings)
   │   │       ← response: content + optional tool_calls
   │   │
   │   ├─ if finish_reason == "tool_calls":
   │   │   ├─ ToolRegistry.execute(name, args)   [5-min timeout]
   │   │   └─ continue loop
   │   └─ else: break
   │
   ├─ 5. SessionManager.save(session_key, session)
   ├─ 6. MemoryConsolidator.maybe_consolidate(session)
   └─ 7. bus.publish_outbound(reply)

ws.rs reads OutboundMessage
   • encode ServerMsg::Response as MessagePack
   • send over WebSocket
          │
          ▼
Client receives ServerMsg::Response → displays to user
```

---

## Wire Protocol

Communication between server and clients uses **MessagePack**-encoded messages over **WebSocket binary frames** (defined in `ombudsman-core/src/protocol.rs`).

### Client → Server (`ClientMsg`)
- `Chat { session_key, content, media }` — send a user message
- `Command { session_key, command }` — send a slash command (`/new`, `/stop`, `/status`, `/sessions`, `/help`)
- `SessionList` — request all sessions
- `Stop { session_key }` — abort the running agent task

### Server → Client (`ServerMsg`)
- `Progress { content, is_tool_hint }` — live update while agent is thinking/tool-calling
- `Response { content, media }` — final agent reply
- `Error { message }` — server-side error
- `Ack { session_key }` — request accepted
- `SessionList { sessions }` — list of sessions with metadata
- `Stopped { session_key }` — task was aborted
- `StatusResponse { content }` — response to `/status` command

See [Wire Protocol](subsystems/wire-protocol.md) for full details.

---

## Concurrency Model

Ombudsman-server is fully async on a Tokio multi-thread runtime.  Each WebSocket connection gets its own `AgentLoop` instance.  Sub-agents spawn additional `tokio::task` instances that share the same `LLMProvider` and `MessageBus`.  The `MessageBus` uses `tokio::sync::mpsc` channels.  Mutable shared state is protected with `tokio::sync::Mutex`.

---

## Workspace Layout

At runtime the server expects a workspace directory (default `~/.ombudsman/workspace/`):

```
~/.ombudsman/
├── config.json             Runtime configuration
└── workspace/
    ├── AGENTS.md           Agent identity / instructions (optional)
    ├── SOUL.md             Personality / values (optional)
    ├── USER.md             User profile (optional)
    ├── TOOLS.md            Tool hints (optional)
    ├── memory/
    │   ├── MEMORY.md       Long-term persistent facts
    │   └── HISTORY.md      Timestamped action log
    ├── sessions/
    │   └── <key>.jsonl     Per-session conversation history
    └── skills/
        └── <name>/
            └── SKILL.md    Skill manifest and instructions
```

---

## Technology Stack

| Layer | Technology |
|---|---|
| Language | Rust 2021 edition |
| Async runtime | Tokio (full features) |
| WebSocket server | axum 0.8 (ws feature) + tower / tower-http |
| WebSocket client | tokio-tungstenite 0.29 (rustls) |
| Wire serialization | MessagePack via rmp-serde |
| GUI toolkit | iced 0.14 (tokio feature) |
| HTTP client | reqwest 0.12 with rustls-tls |
| MCP client | rmcp 1.2 (streamable-http-client-reqwest) |
| Serialization | serde + serde_json |
| CLI parsing | clap 4 (derive feature) |
| Readline editing | rustyline 14 |
| Logging | tracing + tracing-subscriber |
| HTML parsing | scraper 0.21 |
| Error handling | anyhow + thiserror |

---

## Key Design Decisions

### Client / Server Split

The agent logic (LLM calls, tool execution, memory management, session persistence) lives entirely in `ombudsman-server`.  Clients are thin: they encode/decode wire protocol messages and render the UI.  This means:
- Multiple clients can connect to the same running server.
- Clients can be added or swapped without touching agent logic.
- The GUI and CLI share the same `ombudsman-core` protocol library.

### Why MessagePack over the WebSocket?

MessagePack is more compact than JSON for binary-heavy payloads (e.g. media attachments) and faster to encode/decode.  Both client crates and the server share the same `encode_*/decode_*` functions from `ombudsman-core`.

### Why a Message Bus?

The `MessageBus` decouples the WebSocket transport layer from the agent core.  Adding a new channel adapter (Telegram, Discord, etc.) requires only implementing the message-send/receive side of the bus; the agent loop is unchanged.

### Why JSONL for Sessions?

Sessions are stored as newline-delimited JSON.  Each message is an independent JSON object, enabling efficient tail-reads and simple append-only writes without re-serializing the whole file.

### Lazy MCP Connection

MCP servers are not contacted at startup; the first connection attempt happens when the agent processes its first message.  This avoids blocking the startup sequence on remote server availability.

### Provider Abstraction

The `LLMProvider` trait means the agent loop knows nothing about HTTP, API keys, or model naming conventions.  Switching providers is a configuration change, not a code change.  Three concrete implementations cover all providers: `OpenAIProvider` (most), `AnthropicProvider` (native Messages API), and `AzureOpenAIProvider`.


---

## High-Level Component Map

```
┌──────────────────────────────────────────────────────────────────────┐
│                        ombudsman process                             │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                     CLI Interface                            │    │
│  │   (src/cli/commands.rs)                                      │    │
│  │                                                              │    │
│  │  chat | onboard | config | version                          │    │
│  └───────────────────────────┬──────────────────────────────────┘   │
│                              │ InboundMessage                        │
│                              ▼                                       │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                    Message Bus                                 │  │
│  │              (src/bus/mod.rs + events.rs)                      │  │
│  │                                                               │  │
│  │   inbound MPSC channel       outbound MPSC channel            │  │
│  └───────────────┬──────────────────────────────┬───────────────┘  │
│   InboundMessage │                              │ OutboundMessage   │
│                  ▼                              ▲                   │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                      Agent Loop                              │  │
│  │              (src/agent/loop_runner.rs)                      │  │
│  │                                                              │  │
│  │  1. build system prompt (ContextBuilder)                     │  │
│  │  2. load session history (SessionManager)                    │  │
│  │  3. call LLM  ←→  tool execution loop (≤40 iterations)      │  │
│  │  4. consolidate memory (MemoryConsolidator)                  │  │
│  │  5. publish reply                                            │  │
│  └────┬─────────────────────────────────────────────────────────┘  │
│       │                                                              │
│  ┌────┴──────────────┐  ┌───────────────────┐  ┌─────────────────┐ │
│  │  Context Builder  │  │ Session Manager   │  │  LLM Provider   │ │
│  │  (context.rs)     │  │ (session/mod.rs)  │  │  (providers/)   │ │
│  │                   │  │                   │  │                 │ │
│  │ AGENTS.md         │  │ JSONL files       │  │ OpenAI API      │ │
│  │ SOUL.md           │  │ per session key   │  │ Anthropic       │ │
│  │ USER.md           │  │ consolidation     │  │ Deepseek, Groq  │ │
│  │ TOOLS.md          │  │ boundary tracking │  │ Gemini, Ollama… │ │
│  │ MEMORY.md         │  └───────────────────┘  └────────┬────────┘ │
│  │ skills summary    │                                   │          │
│  └───────────────────┘                         tool_calls│          │
│                                                           ▼          │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                     Tool Registry                             │  │
│  │              (src/agent/tools/registry.rs)                    │  │
│  │                                                               │  │
│  │  read_file  write_file  edit_file  list_dir                   │  │
│  │  exec       web_search  web_fetch                             │  │
│  │  mcp_{server}_{tool}  (from MCP servers, added lazily)        │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                          ▲                                           │
│  ┌───────────────────────┴───────────────────────────────────────┐  │
│  │              MCP Subsystem (src/agent/tools/mcp.rs)           │  │
│  │   Lazy HTTP connection via rmcp on first message              │  │
│  │   StreamableHttp + SSE transport  ·  schema normalization     │  │
│  │   McpToolWrapper  ·  connect_mcp_servers                      │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                      │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                  Configuration System                         │  │
│  │               (src/config/)                                   │  │
│  │   ~/.ombudsman/config.json                                    │  │
│  └───────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
```

---

## Data-Flow: Handling One User Message

```
User types message
        │
        ▼
CLI (commands.rs)
  • wraps text in InboundMessage
  • bus.publish_inbound(msg)
        │
        ▼
AgentLoop.run_once()
  │
  ├─ 1. ContextBuilder.build_system_prompt()
  │       reads workspace/AGENTS.md, SOUL.md, USER.md, TOOLS.md
  │       reads memory/MEMORY.md
  │       reads workspace/skills/**/SKILL.md
  │
  ├─ 2. SessionManager.get_history(session_key)
  │       loads JSONL from workspace/sessions/<session_key>.jsonl
  │
  ├─ 3. Build messages[] = [system] + [history] + [new user msg]
  │
  ├─ 4. Loop (up to max_iterations = 40)
  │   │
  │   │   [on first iteration: lazily connect to MCP servers via connect_mcp()]
  │   │
  │   ├─ LLMProvider.chat(model, messages, tools, settings)
  │   │       POST /v1/chat/completions
  │   │       ← response: content + optional tool_calls
  │   │
  │   ├─ Strip <think>…</think> blocks from content
  │   ├─ Append assistant turn to session
  │   │
  │   ├─ if finish_reason == "tool_calls":
  │   │   ├─ for each tool_call:
  │   │   │   ├─ ToolRegistry.execute(name, args)   [5-min timeout]
  │   │   │   └─ append tool result (≤16 000 chars) to session
  │   │   └─ continue loop
  │   │
  │   └─ else: break
  │
  ├─ 5. SessionManager.save(session_key, session)
  │
  ├─ 6. MemoryConsolidator.maybe_consolidate(session)
  │       calls LLM with save_memory tool
  │       writes workspace/memory/MEMORY.md
  │       appends workspace/memory/HISTORY.md
  │
  └─ 7. bus.publish_outbound(reply)

CLI reads OutboundMessage → prints to terminal
```

---

## Concurrency Model

Ombudsman is fully async.  All I/O (HTTP to LLM, filesystem reads/writes, shell subprocess spawning) runs inside a Tokio multi-thread runtime.  The `MessageBus` uses `tokio::sync::mpsc` channels.  Mutable shared state (session store, memory, usage counters, running flag) is protected with `tokio::sync::Mutex`.

The main loop processes one message at a time; there is no concurrent multi-session handling in the current design.

---

## Workspace Layout

At runtime the agent expects a workspace directory (default `~/.ombudsman/workspace/`):

```
~/.ombudsman/
├── config.json             Runtime configuration
└── workspace/
    ├── AGENTS.md           Agent identity / instructions (optional)
    ├── SOUL.md             Personality / values (optional)
    ├── USER.md             User profile (optional)
    ├── TOOLS.md            Tool hints (optional)
    ├── memory/
    │   ├── MEMORY.md       Long-term persistent facts
    │   └── HISTORY.md      Timestamped action log
    ├── sessions/
    │   └── <key>.jsonl     Per-session conversation history
    └── skills/
        └── <name>/
            └── SKILL.md    Skill manifest and instructions
```

---

## Technology Stack

| Layer | Technology |
|---|---|
| Language | Rust 2021 edition |
| Async runtime | Tokio (full features) |
| HTTP client | reqwest 0.12 with rustls-tls |
| MCP client | rmcp 1.2 (streamable-http-client-reqwest feature) |
| Serialization | serde + serde_json |
| CLI parsing | clap 4 (derive feature) |
| Readline editing | rustyline 14 |
| Logging | tracing + tracing-subscriber |
| HTML parsing | scraper 0.21 |
| Date/time | chrono 0.4 |
| Regex | regex 1 |
| Error handling | anyhow + thiserror |

---

## Key Design Decisions

### Why a Message Bus?

The `MessageBus` decouples the transport layer (today: CLI stdin/stdout) from the agent core. Adding a Telegram bot, a REST webhook, or any other channel requires only implementing the message-send/receive side of the bus; the agent loop is unchanged.

### Why JSONL for Sessions?

Sessions are stored as newline-delimited JSON rather than a database.  Each message is an independent JSON object on its own line, which enables:
- Efficient tail-reads (load only the last N messages)
- Simple append-only writes (no re-serializing the whole file)
- Tool-call boundary detection without a full parse

### Why Two Memory Files?

`MEMORY.md` is a living document that the LLM rewrites on each consolidation—it always reflects the current best summary of long-term facts.  `HISTORY.md` is an append-only, grep-searchable log with timestamps, providing auditability without inflating the live prompt.

### Provider Abstraction

The `LLMProvider` trait means the agent loop knows nothing about HTTP, API keys, or model naming conventions.  Switching providers is a configuration change, not a code change.

### Lazy MCP Connection

MCP servers are not contacted at startup; the first connection attempt happens when the agent processes its first message.  This avoids blocking the startup sequence on remote server availability.  If a server is unavailable on the first attempt, `mcp_connected` remains `false` and the connection is retried on the next agent turn.
