# Sub-agent System

**Source:** `src/agent/subagent.rs`, `src/agent/tools/spawn.rs`, `src/agent/tools/message.rs`

The sub-agent system allows the primary agent to spawn background tasks that run concurrently.  A sub-agent executes an independent agent loop in a Tokio task and announces its result via the message bus when complete.

---

## Table of Contents

1. [Overview](#1-overview)
2. [SubagentManager](#2-subagentmanager)
3. [SpawnTool](#3-spawntool)
4. [MessageTool](#4-messagetool)
5. [Integration in AgentLoop](#5-integration-in-agentloop)
6. [Nanobot Comparison](#6-nanobot-comparison)

---

## 1. Overview

When the primary agent calls the `spawn` tool, a lightweight sub-agent loop is started as a background Tokio task.  The primary agent receives an immediate acknowledgement and continues its own conversation.  The sub-agent runs its own iteration of the LLM-tool loop (up to 15 iterations) with a separate, ephemeral tool registry.  When the sub-agent finishes, it publishes its result as an `InboundMessage` on the shared message bus.

In the CLI, a background task drains sub-agent inbound messages and prints them as they arrive.  In a full gateway deployment the message bus would route sub-agent results to the appropriate channel.

```
Primary agent
│
├─ user: "…do X in the background…"
│
├─ LLM decides to call spawn("task": "…")
│
├─ SpawnTool.execute()
│   └─ SubagentManager.spawn(task, session_key, channel, chat_id)
│       └─ tokio::spawn( run_subagent() )   ← returns immediately
│
├─ "Background task started: {label} (ID: {task_id})" → LLM continues
│
└─ [background] run_subagent completes
    └─ bus.send(InboundMessage {sender_id: "subagent", content: result})
        └─ CLI background loop prints result
```

---

## 2. SubagentManager

```rust
pub struct SubagentManager {
    provider:             Arc<dyn LLMProvider>,
    workspace:            PathBuf,
    bus:                  Arc<MessageBus>,
    model:                String,
    web_search_config:    WebSearchConfig,
    web_proxy:            Option<String>,
    exec_config:          ExecToolConfig,
    restrict_to_workspace: bool,
    running_tasks:        Mutex<HashMap<String, AbortHandle>>,
    session_tasks:        Mutex<HashMap<String, HashSet<String>>>,
}
```

The `SubagentManager` is created once in `AgentLoop::new()` and shared (via `Arc`) across the `SpawnTool` and the agent loop.

### spawn()

```rust
pub async fn spawn(
    self: &Arc<Self>,
    task: String,
    label: Option<String>,
    origin_channel: String,
    origin_chat_id: String,
    session_key: String,
) -> String
```

1. Generates an 8-character task ID (`Uuid::new_v4().to_string()[..8]`).
2. Spawns a `tokio::spawn` task that calls `run_subagent()`.
3. Stores the `AbortHandle` in `running_tasks` indexed by task ID.
4. Records the task ID in `session_tasks` for per-session tracking.
5. Returns `"Background task started: {label} (ID: {id})"` immediately.

### run_subagent()

Runs the actual sub-agent logic:

1. Builds an ephemeral `ToolRegistry` with the same built-in tools as the primary agent (filesystem, shell, web).  MCP tools are **not** passed to sub-agents.
2. Constructs a minimal system prompt from `ContextBuilder`.
3. Runs the LLM-tool loop for up to `SUBAGENT_MAX_ITERATIONS = 15` iterations.
4. On completion, publishes:
   ```
   InboundMessage {
       channel: origin_channel,
       chat_id: origin_chat_id,
       content: result,
       sender_id: "subagent",
       session_key: session_key,
   }
   ```
5. Removes the task from `running_tasks` and `session_tasks`.

### Task Cancellation

All running tasks for a given session key can be retrieved via `session_tasks`. Individual tasks can be cancelled via their `AbortHandle`.

---

## 3. SpawnTool

**File:** `src/agent/tools/spawn.rs`

The `SpawnTool` is a `Tool` impl that the primary agent can call to spawn a background sub-agent.

```rust
pub struct SpawnTool {
    manager: Arc<SubagentManager>,
    context: Arc<Mutex<SpawnToolContext>>,
}

pub struct SpawnToolContext {
    pub channel:     String,
    pub chat_id:     String,
    pub session_key: String,
}
```

The `SpawnToolContext` is updated by the `AgentLoop` before each message is processed, so the spawned sub-agent always knows which session and channel originated the request.

### Tool Schema

```json
{
  "name": "spawn",
  "description": "Spawn a subagent to handle a task in the background.",
  "parameters": {
    "type": "object",
    "properties": {
      "task":  {"type": "string", "description": "The task for the subagent"},
      "label": {"type": "string", "description": "Optional short label for display"}
    },
    "required": ["task"]
  }
}
```

---

## 4. MessageTool

**File:** `src/agent/tools/message.rs`

The `MessageTool` allows an agent (primary or sub-agent) to push messages to a chat channel proactively.  This is primarily used by sub-agents to report partial results or by the primary agent when targeting a different channel from the current conversation.

```rust
pub struct MessageTool {
    outbound_tx: mpsc::Sender<OutboundMessage>,
    context:     Arc<Mutex<MessageToolContext>>,
}

pub struct MessageToolContext {
    pub channel: String,
    pub chat_id: String,
}
```

### Tool Schema

```json
{
  "name": "message",
  "description": "Send a message to the user on a chat channel.",
  "parameters": {
    "type": "object",
    "properties": {
      "content": {"type": "string"},
      "channel": {"type": "string", "description": "Defaults to current channel"},
      "chat_id": {"type": "string", "description": "Defaults to current chat"},
      "media":   {"type": "array", "items": {"type": "string"}, "description": "File paths to attach"}
    },
    "required": ["content"]
  }
}
```

The tool uses `mpsc::Sender::try_send()` to publish an `OutboundMessage` non-blockingly.  If the bus is full it returns an error string.

---

## 5. Integration in AgentLoop

`AgentLoop::new()` wires the sub-agent system:

```rust
// Shared SubagentManager
let subagent_manager = Arc::new(SubagentManager::new(
    provider.clone(), workspace, bus.clone(), model,
    web_search_config, web_proxy, exec_config, restrict_to_workspace,
));

// Per-message contexts (updated before each run_once call)
let spawn_ctx   = Arc::new(StdMutex::new(SpawnToolContext::default()));
let message_ctx = Arc::new(StdMutex::new(MessageToolContext::default()));

// Register tools
base_tools.register(Box::new(SpawnTool::new(
    Arc::clone(&subagent_manager), Arc::clone(&spawn_ctx),
)));
base_tools.register(Box::new(MessageTool::new(
    bus.outbound_sender(), Arc::clone(&message_ctx),
)));
```

Before processing each message, the agent loop updates the contexts so `spawn` and `message` know the current channel and session.

The CLI starts a background drain loop at startup:

```rust
tokio::spawn(async move {
    loop {
        if let Ok(msg) = bus_bg.consume_inbound().await {
            if msg.sender_id == "subagent" {
                // process and print sub-agent result
            }
        }
    }
});
```

---

## 6. Nanobot Comparison

| Aspect | nanobot (`ref/nanobot/agent/subagent.py`) | ombudsman (`src/agent/subagent.rs`) |
|---|---|---|
| Spawning mechanism | `asyncio.create_task()` | `tokio::spawn()` ✅ |
| Task tracking | `_running_tasks: dict[str, asyncio.Task]` | `running_tasks: Mutex<HashMap<String, AbortHandle>>` ✅ |
| Session tracking | `_session_tasks: dict[str, set[str]]` | `session_tasks: Mutex<HashMap<String, HashSet<String>>>` ✅ |
| Sub-agent tool set | Full built-in tools (no MCP, no cron) | Full built-in tools (no MCP) ✅ |
| Max iterations | 15 | `SUBAGENT_MAX_ITERATIONS = 15` ✅ |
| Result delivery | Sends InboundMessage via bus | Same ✅ |
| MCP tools in sub-agents | Not passed | Not passed ✅ |
| Cron tool in sub-agents | Not passed | Not applicable (cron not implemented) |
| `spawn` tool | `SpawnTool` Python class | `SpawnTool` Rust struct ✅ |
| `message` tool | `MessageTool` Python class | `MessageTool` Rust struct ✅ |
| sender_id tag | `"subagent"` | `"subagent"` ✅ |
