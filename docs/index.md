# Ombudsman Documentation Index

**Ombudsman** is a lightweight personal AI assistant — a Rust port of the [nanobot](https://github.com/HKUDS/nanobot) project.  It is structured as a **Cargo workspace** with a client/server architecture: `ombudsman-server` runs the AI agent and exposes a WebSocket API, `ombudsman-cli` provides an interactive terminal client, `ombudsman-gui` provides an iced desktop GUI, and `ombudsman-core` holds the shared wire protocol.  Features include a full-featured agentic AI loop with persistent memory, an extensible tool system, multi-provider LLM support (Anthropic native, Azure, OpenAI-compatible), MCP (Model Context Protocol) integration, and background sub-agent execution.

---

## Chapters

| # | Document | Description |
|---|----------|-------------|
| 1 | [Architecture Overview](architecture.md) | High-level design, workspace crates, component map, data-flow diagrams |
| 2 | [Wire Protocol](subsystems/wire-protocol.md) | ClientMsg / ServerMsg MessagePack protocol shared by all clients |
| 3 | [Agent Subsystem](subsystems/agent.md) | Core agent loop, prompt context builder, memory consolidation, skills loader |
| 4 | [Tools Subsystem](subsystems/tools.md) | Tool trait, registry, filesystem tools, shell execution, web search/fetch |
| 5 | [MCP Subsystem](subsystems/mcp.md) | MCP client integration, tool wrapping, schema normalization, lazy connection |
| 6 | [Sub-agent System](subsystems/subagent.md) | Background task execution via SubagentManager, SpawnTool, and MessageTool |
| 7 | [Message Bus](subsystems/message-bus.md) | Async MPSC message bus and event types |
| 8 | [LLM Providers](subsystems/providers.md) | Provider abstraction layer, Anthropic/Azure/OpenAI adapters, registry |
| 9 | [Session Management](subsystems/session.md) | Conversation session, JSONL persistence, consolidation boundaries |
| 10 | [Configuration System](subsystems/config.md) | Configuration schema, paths, file loading |
| 11 | [CLI Client](subsystems/cli.md) | WebSocket CLI client: commands, interactive chat |
| 12 | [GUI Client](subsystems/gui.md) | iced desktop GUI client: MVVM architecture, WS subscription |
| 13 | [Nanobot Comparison](nanobot-comparison.md) | Cross-check against the Python nanobot reference implementation |

---

## Quick Reference

### Source Layout

```
ombudsman/                          Cargo workspace root
├── ombudsman-core/                 Shared wire protocol  →  Chapter 2
│   └── src/
│       ├── lib.rs
│       └── protocol.rs             ClientMsg / ServerMsg / codec
│
├── ombudsman-server/               WebSocket server  →  Chapters 3-10
│   └── src/
│       ├── main.rs                 Entry point (serve subcommand)
│       ├── ws.rs                   axum WS handler
│       ├── builder.rs              Provider + AgentLoop construction
│       ├── agent/                  Agent subsystem  →  Chapter 3
│       │   ├── context.rs
│       │   ├── loop_runner.rs
│       │   ├── memory.rs
│       │   ├── skills.rs
│       │   ├── subagent.rs         Sub-agent system →  Chapter 6
│       │   └── tools/              Tools subsystem  →  Chapter 4
│       │       ├── base.rs
│       │       ├── registry.rs
│       │       ├── filesystem.rs
│       │       ├── shell.rs
│       │       ├── web.rs
│       │       ├── spawn.rs
│       │       ├── message.rs
│       │       └── mcp.rs          MCP subsystem    →  Chapter 5
│       ├── bus/                    Message bus      →  Chapter 7
│       │   ├── mod.rs
│       │   └── events.rs
│       ├── providers/              LLM providers    →  Chapter 8
│       │   ├── base.rs
│       │   ├── openai.rs
│       │   ├── anthropic.rs
│       │   ├── azure.rs
│       │   └── registry.rs
│       ├── session/                Session manager  →  Chapter 9
│       │   └── mod.rs
│       └── config/                 Configuration    →  Chapter 10
│           ├── schema.rs
│           ├── paths.rs
│           └── loader.rs
│
├── ombudsman-cli/                  CLI client  →  Chapter 11
│   └── src/
│       ├── main.rs
│       └── ws_client.rs
│
└── ombudsman-gui/                  GUI client  →  Chapter 12
    └── src/
        ├── main.rs
        ├── app.rs                  iced Application + update/view
        ├── viewmodel.rs            ChatViewModel / SettingsViewModel
        ├── ws_client.rs            iced WS Subscription
        └── view/
            ├── chat.rs
            └── settings_modal.rs
```

### Key Design Principles

- **Client/server split** — Agent logic lives in `ombudsman-server`; clients are thin WebSocket frontends sharing the `ombudsman-core` protocol library.
- **Modularity** — Each subsystem has a clearly bounded responsibility and communicates through well-defined interfaces.
- **Async-first** — Built on Tokio; no blocking operations in the hot path.
- **Safety** — Path sandboxing, shell command guards, and configurable timeouts throughout.
- **Extensibility** — The `Tool` trait and provider registry are open for extension without modifying core logic. MCP integration provides access to the broader ecosystem of MCP-compatible tool servers.
- **Memory-aware** — A two-layer persistent memory system keeps the agent stateful across sessions.
- **Platform-aware** — Runtime OS/arch detection drives platform-specific guidance in the system prompt.
