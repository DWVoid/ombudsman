# CLI Client

**Source:** `ombudsman-cli/`

`ombudsman-cli` is an interactive terminal client for `ombudsman-server`.  It connects to the server via WebSocket using the shared `ombudsman-core` wire protocol, providing a full-featured chat interface with readline history and slash commands.

> **Note:** The CLI is a thin **client** only — it does not run the agent, manage sessions, or hold configuration.  All of that lives in `ombudsman-server`.

---

## Table of Contents

1. [Entry Point and CLI Arguments](#1-entry-point-and-cli-arguments)
2. [WebSocket Client (`WsClient`)](#2-websocket-client-wsclient)
3. [Interactive Chat Loop](#3-interactive-chat-loop)
4. [Slash Commands](#4-slash-commands)

---

## 1. Entry Point and CLI Arguments

**File:** `ombudsman-cli/src/main.rs`

```
ombudsman-cli [OPTIONS]

Options:
  --server <URL>     WebSocket server URL (default: ws://127.0.0.1:7878/ws)
  -m, --model <STR>  Model hint (informational; server uses its configured model)
  -s, --session <KEY> Session key (default: cli:default)
```

The binary initialises tracing, builds a Tokio runtime, and calls `run(args)`.

---

## 2. WebSocket Client (`WsClient`)

**File:** `ombudsman-cli/src/ws_client.rs`

A thin wrapper around a `tokio-tungstenite` connection:

```rust
pub struct WsClient {
    sink:   SplitSink<WebSocketStream<…>, WsMsg>,
    stream: SplitStream<WebSocketStream<…>>,
}

impl WsClient {
    pub async fn connect(url: &str) -> Result<Self>
    pub async fn send(&mut self, msg: ClientMsg) -> Result<()>
    pub async fn recv(&mut self) -> Result<WsEvent>
}
```

### WsEvent

```rust
pub enum WsEvent {
    Message(ServerMsg),
    Disconnected(String),
}
```

Messages are encoded/decoded as MessagePack binary frames using the `encode_client_msg` / `decode_server_msg` functions from `ombudsman-core`.

---

## 3. Interactive Chat Loop

1. Prints the logo, version, server URL, and session key.
2. Spawns a Tokio runtime and calls `run()`.
3. Connects to the server via `WsClient::connect()`.
4. Opens a rustyline `DefaultEditor` for readline-with-history input.
5. Loops:
   - Reads a line from the user.
   - Sends a `ClientMsg::Chat` or `ClientMsg::Command` (see §4).
   - Calls `wait_for_response()`:
     - Streams `ServerMsg::Progress` as live progress lines.
     - Prints `ServerMsg::Response` as the final answer.
     - Handles `ServerMsg::SessionList`, `ServerMsg::Stopped`, `ServerMsg::StatusResponse`.
     - Exits on `WsEvent::Disconnected`.
6. Saves readline history to `$XDG_DATA_HOME/ombudsman-cli/history.txt` on exit.

---

## 4. Slash Commands

| Command | `ClientMsg` sent | Description |
|---|---|---|
| `/new` | `Command { command: "/new" }` | Start a new conversation (server clears session history) |
| `/stop` | `Stop { session_key }` | Abort the currently running agent task |
| `/status` | `Command { command: "/status" }` | Request server status summary |
| `/sessions` | `SessionList` | List all sessions on the server |
| `/help` | _(local)_ | Print help text |
| `exit` / `quit` | _(local)_ | Exit the CLI |

Any other `/`-prefixed input is sent as `ClientMsg::Command` and interpreted by the server.


---

## Table of Contents

1. [Entry Point](#1-entry-point)
2. [Commands](#2-commands)
3. [Interactive Chat](#3-interactive-chat)
4. [Onboarding Wizard](#4-onboarding-wizard)
5. [Auxiliary Commands](#5-auxiliary-commands)

---

## 1. Entry Point

**File:** `src/main.rs`

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // initialise tracing
    // parse CLI args with clap
    // dispatch to command handler
}
```

The binary entry point initialises structured logging (via `tracing-subscriber`) and then hands off to `clap` for argument parsing.

**File:** `src/cli/mod.rs`

Re-exports the `Cli` struct and `Commands` enum from `commands.rs`.

---

## 2. Commands

**File:** `src/cli/commands.rs`

### Top-Level CLI Structure (clap)

```
ombudsman [OPTIONS] [COMMAND]

Options:
  -c, --config <PATH>     Path to config file (default: ~/.ombudsman/config.json)

Commands:
  chat     Start an interactive chat session
  onboard  Initialise the workspace and configure API keys
  config   Show the current configuration
  version  Print the version number
  (none)   Defaults to `chat`
```

When no subcommand is given, ombudsman starts an interactive chat session.

---

## 3. Interactive Chat

### `chat` Command

```
ombudsman chat [OPTIONS]

Options:
  -m, --model   <MODEL>    Override the model (e.g. gpt-4o, claude-opus-4-5)
  -s, --session <KEY>      Override the session key (default: cli:default)
```

### Session Lifecycle

1. **Load config** from `~/.ombudsman/config.json` (or the path given by `--config`).
2. **Resolve the workspace** path (expanding `~`).
3. **Detect or load the LLM provider** using the provider registry.
4. **Construct `AgentLoop`** with all subsystems wired up.
5. **Start the agent loop** as an async background task.
6. **Enter the readline loop** using `rustyline`.

### Readline Loop

```
loop {
    print!("You> ");
    let line = editor.readline("You> ");   // rustyline handles history, editing
    match line {
        Ok(text) => {
            if is_exit_command(&text) { break; }
            bus.publish_inbound(InboundMessage::new("cli", "user", chat_id, &text));
            let reply = bus.consume_outbound().await;
            print_colored_reply(reply.content);
        }
        Err(Interrupted | Eof) => break,
        Err(e) => eprintln!("Error: {}", e),
    }
}
```

### Exit Commands

The following inputs terminate the chat loop:

| Input | Variant |
|---|---|
| `exit` | Bare word |
| `quit` | Bare word |
| `/exit` | Slash-prefixed |
| `/quit` | Slash-prefixed |
| `:q` | Vim-style |
| Ctrl-C | Signal (readline `Interrupted` error) |
| Ctrl-D | EOF (readline `Eof` error) |

### Terminal Output

Replies are printed in colour using the `colored` crate:
- The `"You> "` prompt is displayed in bold.
- Agent replies are displayed in a distinct colour.
- Progress messages (e.g. "thinking…") are displayed in a dimmer colour.

---

## 4. Onboarding Wizard

### `onboard` Command

```
ombudsman onboard [OPTIONS]

Options:
  --wizard    Start the interactive setup wizard (prompts for API keys)
```

Without `--wizard`, `onboard` creates the workspace directory and writes a default `config.json` if one does not already exist.

With `--wizard`, it additionally:
1. Prompts the user to select a provider (OpenAI, Anthropic, OpenRouter, etc.).
2. Reads the API key securely (no echo).
3. Optionally prompts for a preferred model.
4. Writes the updated config to disk.

---

## 5. Auxiliary Commands

### `config`

```
ombudsman config
```

Loads `~/.ombudsman/config.json` and pretty-prints it to stdout.  Useful for quickly verifying that API keys and model settings are correct.

### `version`

```
ombudsman version
```

Prints the binary version string (taken from `Cargo.toml` at compile time via the `CARGO_PKG_VERSION` environment variable).
