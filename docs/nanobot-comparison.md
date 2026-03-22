# Nanobot Comparison

This document cross-checks ombudsman (Rust) against the [nanobot](https://github.com/HKUDS/nanobot) Python reference implementation (source available in the `ref/` directory).  The goal is to identify what has been faithfully ported, what has been adapted, and what is currently absent.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Feature Matrix](#2-feature-matrix)
3. [Subsystem-by-Subsystem Analysis](#3-subsystem-by-subsystem-analysis)
   - [Agent Loop](#31-agent-loop)
   - [Memory System](#32-memory-system)
   - [Skills System](#33-skills-system)
   - [Tools](#34-tools)
   - [Sub-agent Support](#35-sub-agent-support)
   - [MCP Integration](#36-mcp-integration)
   - [LLM Providers](#37-llm-providers)
   - [Session Management](#38-session-management)
   - [Configuration](#39-configuration)
   - [CLI Interface](#310-cli-interface)
   - [Message Bus](#311-message-bus)
4. [Features Present in Nanobot but Absent in Ombudsman](#4-features-present-in-nanobot-but-absent-in-ombudsman)
5. [Features Present in Ombudsman but Different from Nanobot](#5-features-present-in-ombudsman-but-different-from-nanobot)
6. [Summary Table](#6-summary-table)

---

## 1. Overview

| Property | nanobot (Python) | ombudsman (Rust) |
|---|---|---|
| Language | Python 3 | Rust 2021 |
| Distribution | PyPI (`pip install nanobot-ai`) | Single compiled binary |
| Latest release | v0.1.4.post5 (March 2026) | v0.1.0 (initial port) |
| Core agent lines | ~4 000 Python lines | ~7 100 Rust lines (incl. MCP + subagents) |
| Chat channels | 11+ (Telegram, Discord, WhatsApp, …) | CLI client + initial desktop GUI |
| Scheduling | Built-in cron system | Not implemented |
| MCP support | Yes (stdio + HTTP/SSE) | Yes (HTTP/SSE only) |
| Sub-agents | Yes (background tasks) | Yes (background tasks) |
| Heartbeat | Yes (proactive wake-up) | Not implemented |
| Nanobot source reference | — | Available in `ref/` directory |

Ombudsman is a functional port that reproduces the core agentic loop, memory system, skills, built-in tools, provider abstraction, sub-agent background tasks, session persistence, CLI client, initial desktop GUI, and MCP HTTP integration.  It does not yet implement the communication-channel ecosystem (Telegram, Discord, etc.), scheduling, heartbeat, or stdio MCP transport that nanobot provides.

---

## 2. Feature Matrix

| Feature | nanobot | ombudsman | Notes |
|---|---|---|---|
| Core agent loop (LLM ↔ tools) | ✅ | ✅ | Functionally equivalent |
| Think-block stripping | ✅ | ✅ | Both strip `<think>` tags |
| Two-layer memory (MEMORY.md + HISTORY.md) | ✅ | ✅ | Identical design |
| Skills system | ✅ | ✅ | Same SKILL.md frontmatter format |
| read_file / write_file / edit_file / list_dir | ✅ | ✅ | Equivalent |
| Image reading (base64) | ✅ | ✅ | Equivalent |
| exec shell tool | ✅ | ✅ | Same deny-list patterns |
| web_search (Brave + DuckDuckGo) | ✅ | ✅ | Equivalent |
| web_fetch (HTML → text) | ✅ | ✅ | Equivalent |
| Untrusted content banner | ✅ | ✅ | Equivalent |
| OpenAI provider | ✅ | ✅ | Equivalent |
| Anthropic / Claude provider | ✅ | ✅ | Equivalent |
| Deepseek provider | ✅ | ✅ | Equivalent |
| Groq provider | ✅ | ✅ | Equivalent |
| Gemini provider | ✅ | ✅ | Equivalent |
| Moonshot / Kimi provider | ✅ | ✅ | Equivalent |
| Ollama (local) | ✅ | ✅ | Equivalent |
| vLLM / Custom endpoint | ✅ | ✅ | Equivalent |
| OpenRouter | ✅ | ✅ | Equivalent |
| Azure OpenAI | ✅ | ✅ | Equivalent |
| JSONL session persistence | ✅ | ✅ | Equivalent |
| Consolidation boundary tracking | ✅ | ✅ | Equivalent |
| Interactive CLI (readline) | ✅ | ✅ | Both use readline-style editing; CLI is now a WS client |
| Desktop GUI | ❌ | ✅ (initial) | iced-based GUI client (ombudsman-gui) |
| WebSocket server architecture | ❌ (in-process) | ✅ | ombudsman-server exposes ws://host:port/ws |
| Onboarding wizard | ✅ | ✅ | Both have wizard flows |
| Workspace restriction mode | ✅ | ✅ | Equivalent |
| MCP: Streamable HTTP transport | ✅ | ✅ | Equivalent |
| MCP: SSE transport | ✅ | ✅ | Equivalent |
| MCP: Tool wrapping + namespacing | ✅ | ✅ | Same `mcp_{server}_{tool}` scheme |
| MCP: enabledTools filtering | ✅ | ✅ | Equivalent |
| MCP: JSON Schema normalization | ✅ | ✅ | Functionally identical logic |
| MCP: Custom HTTP headers | ✅ | ✅ | Equivalent |
| MCP: Per-tool timeout | ✅ | ✅ | Equivalent |
| MCP: stdio transport | ✅ | ❌ | Not implemented; HTTP-only in Rust |
| MCP: Lazy connection | ❌ | ✅ | Ombudsman connects on first message |
| Sub-agent spawn (`spawn` tool) | ✅ | ✅ | Functionally equivalent |
| Sub-agent message (`message` tool) | ✅ | ✅ | Equivalent |
| Sub-agent max iterations | 15 | 15 ✅ | Same constant |
| MiniMax provider | ✅ | ✅ | OpenAI-compatible via OpenAIProvider |
| AiHubMix gateway | ✅ | ✅ | OpenAI-compatible via OpenAIProvider |
| SiliconFlow gateway | ✅ | ✅ | OpenAI-compatible via OpenAIProvider |
| VolcEngine gateway | ✅ | ✅ | OpenAI-compatible via OpenAIProvider |
| Anthropic native API | ✅ | ✅ | Dedicated AnthropicProvider with thinking mode |
| Azure OpenAI native API | ✅ | ✅ | Dedicated AzureOpenAIProvider |
| web_search: Tavily | ✅ | ❌ | Not implemented |
| web_search: Jina | ✅ | ❌ | Not implemented |
| web_search: SearXNG | ✅ | ❌ | Not implemented |
| Telegram channel | ✅ | ❌ | Not implemented |
| Discord channel | ✅ | ❌ | Not implemented |
| WhatsApp channel | ✅ | ❌ | Not implemented |
| Feishu channel | ✅ | ❌ | Not implemented |
| DingTalk channel | ✅ | ❌ | Not implemented |
| Slack channel | ✅ | ❌ | Not implemented |
| Email channel | ✅ | ❌ | Not implemented |
| QQ channel | ✅ | ❌ | Not implemented |
| WeCom channel | ✅ | ❌ | Not implemented |
| Mochat (Claw IM) channel | ✅ | ❌ | Not implemented |
| Matrix channel | ✅ | ❌ | Not implemented |
| Cron / scheduled tasks | ✅ | ❌ | Not implemented |
| Heartbeat / proactive wake-up | ✅ | ❌ | Not implemented |
| Sub-agent support | ✅ | ✅ | Equivalent; see §3.5 |
| LangSmith tracing | ✅ | ❌ | Not implemented |
| VolcEngine provider | ✅ | ✅ | Via OpenAIProvider + volcengine in registry |
| Qwen provider | ✅ | ❌ | Not in registry |
| Zhipu GLM provider | ✅ | ❌ | Not in registry |
| MiniMax provider | ✅ | ✅ | Via OpenAIProvider + minimax in registry |
| OpenAI Codex OAuth | ✅ | ❌ | Not implemented |
| GitHub Copilot OAuth | ✅ | ❌ | Not implemented |
| Token-based memory trimming | ✅ | ✅ | Both use context_window_tokens |
| Docker support | ✅ | ❌ | No Dockerfile provided |
| Systemd service | ✅ | ❌ | No service file provided |
| Built-in skills (github, weather…) | ✅ | ❌ | Skills dir exists; no bundled skills |

---

## 3. Subsystem-by-Subsystem Analysis

### 3.1 Agent Loop

Both implementations share the same fundamental loop structure:

```
receive message → build prompt → [LLM → tools]* → save session → consolidate memory → send reply
```

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| Max iterations | Configurable (default 40) | Configurable (default 40) ✅ |
| Thinking-mode support | Yes (strips `<think>`) | Yes (strips `<think>`) ✅ |
| Progress streaming | Telegram-style live updates | Progress messages via bus ✅ |
| MCP connection | Eager (at gateway start) | Lazy (on first message) |
| Sub-agent spawning | Yes (`spawn` tool creates background agents) | Yes (`spawn` + `SubagentManager`) ✅ |
| Heartbeat | Periodic proactive messages | Not implemented ❌ |
| Prompt cache optimization | Yes (Anthropic prompt caching) | Not implemented ❌ |

### 3.2 Memory System

Both use the identical two-layer approach with `MEMORY.md` and `HISTORY.md`.

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| File locations | `~/.nanobot/workspace/memory/` | `~/.ombudsman/workspace/memory/` |
| Consolidation trigger | Token-based threshold | Message-count heuristic |
| History entry format | `[YYYY-MM-DD HH:MM]` timestamped paragraph | Same ✅ |
| Max consecutive failures | 3 | 3 ✅ |

### 3.3 Skills System

Both use the same `SKILL.md` frontmatter convention.

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| Bundled skills | github, weather, tmux, schedule, subagent, … | None (directory is present but empty) |
| ClawHub integration | Yes (install skills from a hub) | Not implemented |
| Skill auto-sync | Yes (workspace template sync) | Not implemented |

### 3.4 Tools

Core tools are equivalent.  The main gap is in web search providers.

**Differences:**

| Tool | nanobot | ombudsman |
|---|---|---|
| `read_file` | ✅ | ✅ |
| `write_file` | ✅ | ✅ |
| `edit_file` | ✅ | ✅ |
| `list_dir` | ✅ | ✅ |
| `exec` | ✅ | ✅ |
| `web_search` (Brave) | ✅ | ✅ |
| `web_search` (DuckDuckGo) | ✅ | ✅ |
| `web_search` (Tavily) | ✅ | ❌ |
| `web_search` (Jina) | ✅ | ❌ |
| `web_search` (SearXNG) | ✅ | ❌ |
| `web_fetch` | ✅ | ✅ |
| `spawn` (sub-agent) | ✅ | ✅ |
| `schedule` / cron | ✅ | ❌ |
| MCP tools (via mcpServers) | ✅ | ✅ HTTP only |

**Implementation note on `exec`:** Both implementations use the same deny-list patterns for dangerous commands.  The Rust implementation uses compiled `Regex` objects; the Python implementation uses `re.search`.

### 3.5 Sub-agent Support

Both implementations support spawning background agents via a `spawn` tool.

| Aspect | nanobot | ombudsman |
|---|---|---|
| Manager class | `SubagentManager` | `SubagentManager` ✅ |
| Spawning mechanism | `asyncio.create_task()` | `tokio::spawn()` ✅ |
| Max iterations | 15 | 15 ✅ |
| `spawn` tool | Yes | Yes ✅ |
| `message` tool | Yes | Yes ✅ |
| MCP tools in sub-agents | No | No ✅ |
| Result delivery | InboundMessage via bus | InboundMessage via bus ✅ |
| sender_id tag | `"subagent"` | `"subagent"` ✅ |
| Session-level task tracking | Yes | Yes ✅ |
| Cron tool in sub-agents | No | N/A (cron not implemented) |

Sub-agents do not inherit MCP tools from the primary agent — they build their own ephemeral tool registry with only built-in tools.  This matches nanobot's behaviour.

See [Sub-agent System](subsystems/subagent.md) for the full implementation details.

### 3.6 MCP Integration

MCP integration is now implemented in ombudsman for HTTP-based transports.

**Comparison:**

| Aspect | nanobot (`ref/nanobot/agent/tools/mcp.py`) | ombudsman (`src/agent/tools/mcp.rs`) |
|---|---|---|
| MCP library | Python `mcp` SDK | Rust `rmcp` crate |
| Streamable HTTP | ✅ | ✅ |
| SSE transport | ✅ | ✅ |
| stdio transport | ✅ | ❌ Not supported |
| Connection lifecycle | `AsyncExitStack` (context manager) | `McpSession` RAII struct |
| Lazy connection | No (eager at gateway start) | Yes (on first message) |
| Schema normalization | `_normalize_schema_for_openai()` | `normalize_schema_for_openai()` |
| Normalization logic | Functionally identical | Functionally identical ✅ |
| Tool namespacing | `mcp_{server}_{tool}` | `mcp_{server}_{tool}` ✅ |
| enabledTools filtering | ✅ | ✅ |
| Per-tool timeout | ✅ (default 30s) | ✅ (default 30s) |
| Custom headers | ✅ | ✅ |
| Unmatched tool warning | ✅ | ✅ |
| Transport auto-detection | URL ending `/sse` → SSE | Same convention ✅ |

**Key architectural difference:** nanobot uses Python's `AsyncExitStack` to manage the lifecycle of MCP sessions via context managers; all sessions are opened eagerly when the gateway starts.  Ombudsman uses a `McpSession` RAII struct that holds an `rmcp::RunningService` — dropping the struct closes the connection.  Connections are opened lazily on the first agent message, which improves startup latency.

**stdio not supported:** Nanobot supports `stdio` transport to spawn local MCP server processes via `StdioServerParameters`.  Ombudsman explicitly does not support this.  Entries without a `url` are skipped with a warning message.  Only `sse` and `streamableHttp` are accepted.

### 3.7 LLM Providers

**Key differences:**

| Provider | nanobot | ombudsman |
|---|---|---|
| OpenAI | ✅ | ✅ native (OpenAI-compatible HTTP) |
| Anthropic / Claude | ✅ | ✅ native Messages API (AnthropicProvider) |
| Azure OpenAI | ✅ | ✅ native (AzureOpenAIProvider) |
| Deepseek, Groq, Gemini, Moonshot, Ollama, vLLM, Custom | ✅ | ✅ |
| OpenRouter | ✅ | ✅ |
| AiHubMix | ✅ | ✅ |
| SiliconFlow | ✅ | ✅ |
| VolcEngine | ✅ | ✅ |
| MiniMax | ✅ | ✅ |
| Qwen (Alibaba DashScope) | ✅ | ❌ |
| Zhipu GLM | ✅ | ❌ |
| BytePlus | ✅ | ❌ |
| VolcEngine Coding Plan / BytePlus Coding Plan | ✅ | ❌ |
| OpenAI Codex (OAuth) | ✅ | ❌ |
| GitHub Copilot (OAuth) | ✅ | ❌ |
| Provider abstraction layer | Python class hierarchy | Rust `async_trait` ✅ |
| Provider used internally (most) | LiteLLM (Python lib) | Direct HTTP (reqwest) |
| Anthropic provider | LiteLLM + native (dual) | Native Messages API only |
| Azure OpenAI provider | Dedicated class + LiteLLM | Dedicated AzureOpenAIProvider |

**Architecture difference:** nanobot delegates most HTTP calls to [LiteLLM](https://github.com/BerriAI/litellm).  Ombudsman implements all HTTP protocols directly in Rust (`openai.rs`, `anthropic.rs`, `azure.rs`).  This eliminates the Python interpreter and LiteLLM as runtime dependencies, but means every new non-OpenAI-compatible provider requires a Rust implementation.  Providers that use the OpenAI-compatible format (most of the registry) are covered by a single `OpenAIProvider` instance.

### 3.8 Session Management

Both use JSONL files for persistence with per-session files.

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| File format | JSONL | JSONL ✅ |
| Session key scheme | `{channel}:{chat_id}` | `{channel}:{chat_id}` ✅ |
| Legal-start detection | Yes | Yes ✅ |
| Multi-instance safety | File locking / atomic writes | Single-process (no locking) |

### 3.9 Configuration

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| Base directory | `~/.nanobot/` | `~/.ombudsman/` |
| Config file | `~/.nanobot/config.json` | `~/.ombudsman/config.json` |
| Config schema | Pydantic models | Serde structs |
| JSON key naming | `camelCase` | `camelCase` ✅ |
| MCP server config | Full runtime support (incl. stdio) | Full runtime support (HTTP only) |
| MCP `mcpServers` key | Same | Same ✅ |
| Channels config | Rich per-channel config | Stub (only `sendProgress`, `sendToolHints`) |

### 3.10 CLI Interface

Ombudsman's CLI has been restructured as a **thin WebSocket client** (`ombudsman-cli`) that connects to `ombudsman-server`.

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| Architecture | All-in-one (CLI + agent in same process) | Client/server (CLI is a WS client) |
| Main command name | `nanobot` | `ombudsman-cli` |
| Connect to server | N/A | `--server ws://host:port/ws` |
| Chat | `nanobot agent` | `ombudsman-cli` (connects then chatting) |
| Start gateway | `nanobot gateway` | `ombudsman-server serve` |
| Config display | `nanobot config` | Server-side; `/status` command |
| Onboarding | `nanobot onboard [--wizard]` | Server-side (if present) |
| Provider login (OAuth) | `nanobot provider login` | Not implemented |
| Status display | `nanobot status` | `/status` command over WS |
| Readline library | Python `readline` + prompt_toolkit | Rust `rustyline` ✅ |
| Coloured output | Yes | Yes ✅ |
| Session listing | N/A | `/sessions` command |

### 3.10b Desktop GUI

Nanobot has no desktop GUI.  Ombudsman includes an initial `ombudsman-gui` built with iced 0.14, featuring a chat panel and settings modal over WebSocket.

| Aspect | nanobot | ombudsman |
|---|---|---|
| Desktop GUI | ❌ | ✅ initial (iced 0.14) |
| GUI architecture | N/A | MVVM (ViewModel + iced update/view loop) |
| GUI WS subscription | N/A | `Subscription::run_with` + `ws_stream` |
| Settings modal | N/A | Server URL, session key, model hint |

### 3.11 Message Bus

**Differences:**

| Aspect | nanobot | ombudsman |
|---|---|---|
| Transport abstraction | Python asyncio Queue + channel plugins | Tokio MPSC + message structs ✅ |
| Plugin system | Channel plugin guide; runtime-loaded | Not implemented |
| Multiple simultaneous channels | Yes (gateway mode) | Not implemented (bus exists, no gateway) |

---

## 4. Features Present in Nanobot but Absent in Ombudsman

### 4.1 Chat Channel Integrations

Nanobot ships with 11+ chat channel adapters (Telegram, Discord, WhatsApp, Feishu, DingTalk, Slack, Email, QQ, WeCom, Mochat, Matrix).  Ombudsman has the `MessageBus` abstraction in place, but no concrete channel implementations beyond CLI.

**Impact:** Ombudsman can only be used interactively from a terminal.  Deploying it as a persistent bot on any messaging platform requires implementing a channel adapter.

### 4.2 MCP stdio Transport

Nanobot supports `stdio` transport for spawning local MCP server processes (`StdioServerParameters` via `mcp.client.stdio`).  Ombudsman only supports HTTP-based transports (Streamable HTTP and SSE).

**Impact:** MCP servers that are only available as local executables (e.g., installed via `npx` or `uvx`) cannot be used with ombudsman.  Cloud-hosted or locally-bound HTTP MCP servers work normally.

### 4.3 Cron / Scheduled Tasks

Nanobot includes a built-in cron system for time-triggered agent actions (e.g., "remind me every Monday at 9 AM").  This requires persistent scheduling state and a background timer loop.

**Impact:** Ombudsman cannot perform proactive, time-based tasks.

### 4.4 Heartbeat / Proactive Messages

Nanobot has a heartbeat component that wakes the agent periodically to send status updates or check in with users even when no message has been received.

**Impact:** Ombudsman is purely reactive; it only acts when a message is received.

### 4.5 Additional Web Search Providers

Nanobot supports Tavily, Jina, and SearXNG in addition to Brave and DuckDuckGo.

### 4.6 Additional LLM Providers

Nanobot supports Qwen (DashScope), Zhipu GLM, BytePlus, OpenAI Codex (OAuth), and GitHub Copilot (OAuth).  VolcEngine and MiniMax are now supported in ombudsman.  The remaining gaps are China-market providers (Qwen, Zhipu) and OAuth-authenticated endpoints (Codex, GitHub Copilot).

### 4.7 Built-in Skills

Nanobot ships with a library of ready-to-use skills (github, weather, tmux, schedule, subagent, etc.) in a `skills/` directory.  Ombudsman has the skills infrastructure but no bundled skills.

### 4.8 Docker and Systemd Support

Nanobot provides a `Dockerfile`, `docker-compose.yml`, and a documented `systemd` service configuration for server deployment.  Ombudsman has none of these.

---

## 5. Features Present in Ombudsman but Different from Nanobot

### 5.1 Client / Server Architecture

Nanobot is a single all-in-one process.  Ombudsman splits into a server (`ombudsman-server`) and thin clients (`ombudsman-cli`, `ombudsman-gui`).  The server exposes a WebSocket API at `ws://host:port/ws`; clients communicate using MessagePack-encoded messages defined in `ombudsman-core`.  This design means multiple clients can connect concurrently, and new frontends can be added without touching the agent.

### 5.2 Desktop GUI

Nanobot has no desktop GUI.  Ombudsman ships an initial iced-based (`ombudsman-gui`) with a chat panel, MVVM architecture, and a settings modal for configuring the server URL and session key.  The GUI connects to the same WebSocket server as the CLI.

### 5.3 Direct HTTP (No LiteLLM Dependency)

Ombudsman calls LLM APIs directly via `reqwest` rather than routing through LiteLLM.  This eliminates the Python interpreter and LiteLLM as runtime dependencies, making the binary fully self-contained.  The trade-off is that adding support for a non-OpenAI-compatible provider requires implementing the HTTP protocol in Rust.

### 5.4 Compiled Binary

The Rust binary has no runtime dependency on a Python interpreter or virtual environment.  This simplifies deployment on systems where Python is unavailable or where startup latency matters.

### 5.5 Type Safety and Memory Safety

Rust's ownership and type systems prevent entire classes of bugs common in Python: use-after-free, data races, null-pointer dereferences, and unhandled errors.  The `anyhow`/`thiserror` error model requires explicit error handling throughout the codebase.

### 5.6 Async Model

Both projects are async, but the underlying runtimes differ: Python's `asyncio` vs Rust's Tokio.  Tokio provides true multi-threading for async tasks, whereas CPython's GIL limits Python's concurrency for CPU-bound work.

### 5.7 Lazy MCP Connection

Nanobot opens all MCP connections eagerly at gateway startup.  Ombudsman defers MCP connections until the first message is processed.  This improves startup latency when MCP servers are slow to respond and enables the agent to recover from transient failures without a full restart.

### 5.8 rmcp vs mcp SDK

Ombudsman uses the [`rmcp`](https://crates.io/crates/rmcp) Rust crate (official Rust MCP SDK) in client mode.  Nanobot uses the Python `mcp` SDK.  Both implement the same MCP specification; the Rust SDK does not support stdio transport in the features used by ombudsman.

---

## 6. Summary Table

| Category | Implemented in Ombudsman | Gap vs Nanobot |
|---|---|---|
| Core agent loop | ✅ Complete | None |
| Memory (MEMORY.md + HISTORY.md) | ✅ Complete | Consolidation trigger heuristic differs |
| Skills system | ✅ Infrastructure | No bundled skills |
| Filesystem tools | ✅ Complete | None |
| Shell tool | ✅ Complete | None |
| Web tools (Brave + DuckDuckGo) | ✅ Complete | Tavily / Jina / SearXNG missing |
| LLM providers (core + VolcEngine/MiniMax/AiHubMix/SiliconFlow) | ✅ Complete | Qwen, Zhipu, BytePlus, OAuth providers missing |
| Anthropic native API (thinking mode) | ✅ Complete | None |
| Azure OpenAI native API | ✅ Complete | None |
| Session persistence | ✅ Complete | No multi-instance file locking |
| Configuration system | ✅ Complete | Channel config is a stub |
| Client/server WebSocket architecture | ✅ Complete (different from nanobot) | — |
| CLI client (WS) | ✅ Complete | No gateway / onboard / provider-login subcommands |
| Desktop GUI (iced) | ✅ Initial | Chat panel + settings; no history view, no file attach UI |
| Message bus abstraction | ✅ Infrastructure | No channel plugins beyond CLI/GUI |
| Wire protocol (MessagePack over WS) | ✅ Complete | None |
| MCP: Streamable HTTP + SSE | ✅ Complete | stdio transport not supported |
| MCP: Tool wrapping + filtering | ✅ Complete | None |
| MCP: Schema normalization | ✅ Complete | None |
| MCP: Lazy connection | ✅ Better than nanobot | — |
| Sub-agents (spawn + message tools) | ✅ Complete | Cron tool not available in sub-agents (cron not impl.) |
| Chat channel integrations (Telegram…) | ❌ Not started | Telegram, Discord, WhatsApp, and 8+ others |
| Cron / scheduling | ❌ Not started | — |
| Heartbeat | ❌ Not started | — |
| Qwen / Zhipu / BytePlus providers | ❌ Not in registry | — |
| OAuth providers (Codex, Copilot) | ❌ Not started | — |
| Docker / systemd | ❌ Not provided | — |
