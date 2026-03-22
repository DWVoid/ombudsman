# Agent Subsystem

**Source:** `src/agent/`

The agent subsystem is the brain of ombudsman. It owns the end-to-end processing pipeline: reading user messages, assembling prompts, driving the LLM ↔ tool loop, managing memory, and loading skills.

---

## Table of Contents

1. [AgentLoop](#1-agentloop)
2. [ContextBuilder](#2-contextbuilder)
3. [MemoryConsolidator and MemoryStore](#3-memoryconsolidator-and-memorystore)
4. [SkillsLoader](#4-skillsloader)

---

## 1. AgentLoop

**File:** `src/agent/loop_runner.rs`

### Responsibilities

- Receives `InboundMessage` objects from the `MessageBus`.
- Builds the full message array (system prompt + session history + new user message).
- Drives the LLM ↔ tool iteration loop until the model returns a final answer or the iteration limit is reached.
- Saves the updated session to disk.
- Triggers optional memory consolidation.
- Publishes an `OutboundMessage` back onto the `MessageBus`.

### Struct Fields

```rust
pub struct AgentLoop {
    bus: Arc<MessageBus>,
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    model: String,
    max_iterations: u32,          // default 40
    context_window_tokens: u32,   // default 65 536
    context: ContextBuilder,
    sessions: Mutex<SessionManager>,
    /// Built-in tools (read/write/exec/web…); never modified after construction.
    base_tools: ToolRegistry,
    /// Combined registry (base + MCP tools); populated lazily on first message.
    tools: Mutex<Option<Arc<ToolRegistry>>>,
    memory: Mutex<MemoryConsolidator>,
    channels_config: ChannelsConfig,
    start_time: Instant,
    last_usage: Mutex<HashMap<String, u64>>,
    running: Mutex<bool>,
    /// MCP server configurations from config.
    mcp_servers: HashMap<String, McpServerConfig>,
    /// Active MCP sessions kept alive to maintain connections.
    mcp_sessions: Mutex<Vec<McpSession>>,
    /// Whether MCP connection has been attempted.
    mcp_connected: Mutex<bool>,
}
```

### Construction

`AgentLoop::new()` accepts all runtime configuration and wires every subsystem:

1. Determines the model name (from config or provider default).
2. Creates `ContextBuilder`, `SessionManager`, and `MemoryConsolidator`.
3. Registers built-in tools in `base_tools` based on configuration flags:
   - Always registered: `read_file`, `write_file`, `edit_file`, `list_dir`.
   - Registered if `exec.enable == true`: `exec`.
   - Always registered: `web_search`, `web_fetch`.
4. Stores MCP server configs for lazy connection on first message.
5. The `tools` field starts as `None`; it is populated by `connect_mcp()` on first use.

### The Processing Loop

```
run_once(inbound: InboundMessage)
│
├─ derive session_key from message
├─ load or create Session
├─ build system prompt via ContextBuilder
├─ assemble messages[] = [system, ...history, new_user_msg]
├─ lazily connect to MCP servers (no-op if already connected or none configured)
│
├─ for iteration in 0..max_iterations:
│   ├─ send messages + tool_definitions to LLM
│   ├─ strip <think>…</think> from response content
│   ├─ append assistant turn to session
│   │
│   ├─ if finish_reason == "tool_calls":
│   │   ├─ for each tool_call (id, name, args):
│   │   │   ├─ ToolRegistry.execute(name, args)  ← 5-min timeout
│   │   │   ├─ truncate result to TOOL_RESULT_MAX_CHARS (16 000)
│   │   │   └─ append tool result message to session
│   │   └─ continue loop
│   │
│   └─ else: break  ← model returned a final answer
│
├─ save session
├─ maybe trigger memory consolidation
└─ publish OutboundMessage
```

### Tool Result Capping

Tool outputs are capped at **16 000 characters** before being added to the session. This prevents very large tool outputs (e.g., a huge directory listing) from exhausting the context window.

### Think-Block Stripping

Some reasoning models emit internal reasoning inside `<think>…</think>` XML blocks. The agent strips these before storing the assistant turn and before sending the reply to the user.

### Progress and Hint Messages

When `channels_config.send_progress` is enabled the agent publishes intermediate `OutboundMessage` events:
- A "thinking…" notice before calling the LLM.
- Tool-hint notices (when `send_tool_hints` is also enabled) listing which tool is being called.

### Token-Usage Tracking

The agent records cumulative token usage per session in `last_usage` and logs it via `tracing::info`. This is informational only; it does not affect routing decisions.

### MCP Connection Management

See [MCP Subsystem](mcp.md) §7 for the detailed description of lazy connection, registry cloning, and session lifecycle.  The three key methods are:

| Method | Description |
|---|---|
| `connect_mcp()` | One-shot lazy connector; no-op if already connected or no servers configured |
| `get_tools()` | Returns the combined (base + MCP) `Arc<ToolRegistry>` |
| `close_mcp()` | Drops all sessions and resets state; next message retriggers connection |

---

## 2. ContextBuilder

**File:** `src/agent/context.rs`

### Responsibilities

Assembles the system prompt that is sent to the LLM at the start of every request.

### Prompt Structure

The prompt is assembled in the following order, with each section separated by `\n\n---\n\n`:

| Section | Source | Condition |
|---|---|---|
| Agent identity | Hard-coded template | Always present |
| Bootstrap files | `workspace/{AGENTS,SOUL,USER,TOOLS}.md` | If file exists |
| Memory context | `workspace/memory/MEMORY.md` | If non-empty |
| Active skills | `workspace/skills/<name>/SKILL.md` where `mode == always` | If any exist |
| Skills summary | Generated from all available skills | If any skills exist |

### Agent Identity Block

The identity block includes:
- Name and emoji (`ombudsman 🐈`).
- A brief description ("Rust port of nanobot").
- Current workspace path.
- Runtime context tag (OS, architecture) — marked as metadata, not instructions.
- Platform policy: Windows vs POSIX guidance (selected at compile time via `cfg!(windows)`).

### Bootstrap Files

Files are loaded from the workspace root. Any subset of them may exist; missing files are silently skipped.

| File | Typical content |
|---|---|
| `AGENTS.md` | Identity overrides, behavioural guidelines |
| `SOUL.md` | Personality and values |
| `USER.md` | User profile and preferences |
| `TOOLS.md` | Tool usage hints |

### Memory Context

Delegates to `MemoryStore.get_memory_context()`, which reads `memory/MEMORY.md`. If the file is empty or absent, this section is omitted.

### Skills Sections

Two separate sections are generated:
1. **Active skills** — Full content of `SKILL.md` files whose frontmatter sets `mode: always`. These skills are loaded into the context unconditionally.
2. **Skills summary** — A short listing of all available skills so the model knows it can fetch them on demand using `read_file`.

---

## 3. MemoryConsolidator and MemoryStore

**File:** `src/agent/memory.rs`

### Overview

Memory provides cross-session persistence. Without it, the agent starts each new session with no knowledge of past interactions.

The system uses two complementary files:

| File | Purpose | Access pattern |
|---|---|---|
| `memory/MEMORY.md` | Long-term facts, preferences, knowledge | Full overwrite on each consolidation |
| `memory/HISTORY.md` | Timestamped action log | Append-only |

### MemoryStore

Low-level file operations.

```rust
pub struct MemoryStore {
    memory_dir: PathBuf,
    memory_file: PathBuf,
    history_file: PathBuf,
    consecutive_failures: u32,
}
```

Key methods:
- `read_long_term() → String` — reads `MEMORY.md`.
- `write_long_term(content)` — atomically overwrites `MEMORY.md`.
- `append_history(entry)` — appends an entry to `HISTORY.md` with a trailing newline.
- `get_memory_context() → String` — returns the full content of `MEMORY.md` for prompt injection.

### MemoryConsolidator

High-level consolidation logic.

```rust
pub struct MemoryConsolidator {
    store: MemoryStore,
    provider: Option<Arc<dyn LLMProvider>>,
    context_window_tokens: u32,
}
```

#### Consolidation Trigger

Consolidation is triggered after each complete agent turn.  The consolidator slices the session messages since `last_consolidated` and, if there is sufficient new content, invokes the LLM with a specialised prompt.

#### The `save_memory` Pseudo-Tool

The consolidation LLM call uses a single synthetic tool called `save_memory`.  The model is instructed to call it with two arguments:

| Argument | Description |
|---|---|
| `history_entry` | A paragraph summary starting with `[YYYY-MM-DD HH:MM]` |
| `memory_update` | The full updated `MEMORY.md` content |

When the model calls this tool, the consolidator:
1. Appends `history_entry` to `HISTORY.md`.
2. Overwrites `MEMORY.md` with `memory_update`.
3. Advances `session.last_consolidated`.

#### Failure Handling

If the LLM fails to call `save_memory` or returns an error, the failure is counted.  After `MAX_FAILURES` (3) consecutive failures the consolidator logs a warning and stops retrying for that session to avoid a runaway loop.

---

## 4. SkillsLoader

**File:** `src/agent/skills.rs`

### Purpose

Skills are optional, named capability packages that live in the workspace.  They allow the agent (and users) to extend its behaviour without modifying source code.

### Skill Layout

```
workspace/skills/
└── <skill-name>/
    └── SKILL.md       Frontmatter + instructions
```

### Frontmatter Schema

`SKILL.md` files begin with a YAML-like frontmatter block delimited by `---`:

```yaml
---
mode: always          # or "available"
category: coding      # arbitrary grouping label
description: "..."    # one-line description shown in the skills summary
available: true       # if false, skill is hidden from the agent
---
```

| Field | Values | Meaning |
|---|---|---|
| `mode` | `always` | Load full content into every system prompt |
| `mode` | `available` | List in skills summary; loaded on demand by the agent via `read_file` |
| `available` | `false` | Exclude entirely from the system prompt |

### Loading Logic

1. `SkillsLoader::load_all()` enumerates `workspace/skills/*/SKILL.md`.
2. Each file's frontmatter is parsed to produce a `SkillMeta` struct.
3. Skills with `available: false` are ignored.
4. `get_always_skills()` returns the subset with `mode: always`.
5. `load_skills_for_context(names)` reads and returns the full Markdown content of the named skills.
6. `build_skills_summary()` generates the `# Skills` listing for the prompt.
