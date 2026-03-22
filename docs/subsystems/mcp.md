# MCP (Model Context Protocol) Subsystem

**Source:** `src/agent/tools/mcp.rs`  
**Dependency:** `rmcp` crate (v1.2, HTTP transport features only)

The MCP subsystem connects ombudsman to external [Model Context Protocol](https://modelcontextprotocol.io/) servers over HTTP and exposes their tools natively inside the agent's tool registry.

---

## Table of Contents

1. [Overview and Scope](#1-overview-and-scope)
2. [Transport Support](#2-transport-support)
3. [McpToolWrapper](#3-mcptoolwrapper)
4. [Schema Normalization](#4-schema-normalization)
5. [connect_mcp_servers](#5-connect_mcp_servers)
6. [Tool Filtering (enabledTools)](#6-tool-filtering-enabledtools)
7. [Lazy Connection in AgentLoop](#7-lazy-connection-in-agentloop)
8. [McpSession Lifetime](#8-mcpsession-lifetime)
9. [Configuration Reference](#9-configuration-reference)
10. [Nanobot Comparison](#10-nanobot-comparison)

---

## 1. Overview and Scope

MCP is an open protocol that allows AI agents to call tools exposed by external servers. Ombudsman integrates as an **MCP client**: it connects to one or more MCP servers, discovers their tools, and wraps each one as a native `Tool` trait object that participates in the existing `ToolRegistry` pipeline.

The implementation uses the [`rmcp`](https://crates.io/crates/rmcp) Rust crate (official Rust MCP SDK) in client mode.

**What is supported:**

| Feature | Status |
|---|---|
| Streamable HTTP transport | ✅ Supported |
| SSE transport (legacy `/sse` endpoint) | ✅ Supported (via StreamableHttpClientTransport) |
| Custom HTTP headers per server | ✅ Supported |
| Per-tool enable/disable filtering | ✅ Supported |
| Lazy connection (first message) | ✅ Supported |
| Multiple simultaneous MCP servers | ✅ Supported |
| stdio transport | ❌ Explicitly not supported |
| Tool timeout per server | ✅ Configurable |

**What is not supported:**

- `stdio` transport (spawning local processes). The architecture requires in-process async tasks; launching and managing child processes for MCP is out of scope. Any server with no `url` is skipped with a warning.

---

## 2. Transport Support

Both MCP transport protocols are handled by `rmcp`'s `StreamableHttpClientTransport`:

| Protocol | Config `type` value | URL convention |
|---|---|---|
| Streamable HTTP | `"streamableHttp"` | Any URL not ending with `/sse` |
| Legacy SSE | `"sse"` | URLs ending with `/sse` |

### Auto-detection

When `type` is omitted from the server config, the transport is inferred from the URL:

```rust
fn detect_transport_type(url: &str) -> McpTransportType {
    if url.trim_end_matches('/').ends_with("/sse") {
        McpTransportType::Sse
    } else {
        McpTransportType::StreamableHttp
    }
}
```

Both protocols are ultimately handled the same way at the `rmcp` transport layer: `StreamableHttpClientTransport` supports both legacy SSE endpoints and the newer streamable HTTP protocol.

---

## 3. McpToolWrapper

Each tool discovered from an MCP server is wrapped in an `McpToolWrapper` that implements the `Tool` trait.

```rust
pub struct McpToolWrapper {
    name:          String,  // "mcp_{server}_{tool_name}"
    original_name: String,  // raw MCP tool name
    description:   String,
    parameters:    Value,   // OpenAI-normalised JSON Schema
    peer:          Arc<rmcp::Peer<rmcp::RoleClient>>,
    tool_timeout:  u64,     // seconds
}
```

### Namespacing

Wrapped tool names are prefixed with `mcp_{server_name}_` to prevent collisions with built-in tools:

```
MCP server "filesystem" with tool "read" → "mcp_filesystem_read"
MCP server "github"     with tool "search_repos" → "mcp_github_search_repos"
```

The `original_name` is preserved separately and used when calling the server.

### Clone Support

`McpToolWrapper` implements `clone_box()` (required by the updated `Tool` trait) so the base registry can be cheaply cloned before MCP tools are appended. The `peer` is wrapped in `Arc` so cloning is O(1) — multiple wrappers share the same connection.

### Execution

```
tool.execute(args)
│
├─ Convert args HashMap → serde_json Map
├─ Build CallToolRequestParams(original_name, arguments)
├─ tokio::time::timeout(tool_timeout, peer.call_tool(params))
│
├─ Timeout  → "(MCP tool call timed out after Ns)"
├─ Error    → "(MCP tool call failed: <error>)"
└─ Success  → join content blocks with "\n"
              "(no output)" if empty
```

---

## 4. Schema Normalization

MCP tool schemas use JSON Schema conventions that are not always compatible with OpenAI's function-calling format.  `normalize_schema_for_openai()` bridges the gap by converting the following patterns:

### Nullable Type Arrays

```json
// MCP / JSON Schema
{"type": ["string", "null"]}

// After normalization
{"type": "string", "nullable": true}
```

### Nullable anyOf / oneOf

```json
// MCP / JSON Schema
{"anyOf": [{"type": "string"}, {"type": "null"}], "description": "optional value"}

// After normalization
{"type": "string", "description": "optional value", "nullable": true}
```

The `anyOf`/`oneOf` key is removed; the single non-null branch is merged into the parent object.

Non-nullable unions (two or more non-null types) are left untouched.

### Object Defaults

Object-type schemas automatically receive:

```json
{"properties": {}, "required": []}
```

if those keys are absent.

### Recursive Application

`normalize_schema_for_openai` is applied recursively to:
- Each property in `"properties"`
- `"items"` in array schemas

---

## 5. connect_mcp_servers

```rust
pub async fn connect_mcp_servers(
    mcp_servers: &HashMap<String, McpServerConfig>,
    registry: &mut ToolRegistry,
) -> Vec<McpSession>
```

This function iterates over all configured MCP servers and for each one:

1. **Validates URL** — skips entries with an empty URL (with a warning).
2. **Detects transport** — uses explicit `type` or auto-detects from the URL.
3. **Builds headers** — parses custom HTTP header name/value strings.
4. **Creates transport** — instantiates `StreamableHttpClientTransport` via `rmcp`.
5. **Establishes connection** — calls `().serve(transport).await` to perform the MCP handshake.
6. **Lists tools** — calls `peer.list_all_tools()`.
7. **Applies tool filter** — registers only tools allowed by `enabled_tools` (see §6).
8. **Returns sessions** — returns a `Vec<McpSession>` that must be kept alive.

Failures at any step are logged as warnings and skipped; the remaining servers continue to connect. This means a broken MCP server does not prevent the agent from starting.

---

## 6. Tool Filtering (enabledTools)

The `enabled_tools` field in `McpServerConfig` controls which of a server's tools are registered:

| `enabled_tools` value | Behaviour |
|---|---|
| `["*"]` (default) | All tools from the server are registered |
| `["search", "lookup"]` | Only `search` and `lookup` are registered |
| `["mcp_server_search"]` | Accepts namespaced names too |
| `[]` | No tools are registered |

After registration, any names in `enabled_tools` that matched no tool are logged as a warning with the list of available raw and wrapped names to help diagnose typos.

---

## 7. Lazy Connection in AgentLoop

MCP connections are established **lazily on the first message** rather than at construction time. This avoids delaying startup when the configured MCP servers are slow to respond, and allows the agent to recover from transient connectivity failures on subsequent messages.

```rust
impl AgentLoop {
    async fn connect_mcp(&self) {
        // No-op if no servers configured
        // No-op if already connected
        // Clones base_tools registry, appends MCP tools, stores result
        // Sets mcp_connected = true
    }

    async fn get_tools(&self) -> Arc<ToolRegistry> {
        // Returns cached combined registry after connection
        // Falls back to base tools only if no MCP configured
    }
}
```

### Registry Architecture

The `AgentLoop` maintains two registry fields:

```rust
base_tools: ToolRegistry,               // built-in tools (read/write/exec/web…)
tools: Mutex<Option<Arc<ToolRegistry>>>, // combined after MCP connection
```

On first message:
1. `base_tools.clone_registry()` clones all built-in tools via `clone_box()`.
2. `connect_mcp_servers()` appends `McpToolWrapper` objects to the clone.
3. The combined registry is stored in `tools` behind an `Arc`.
4. All subsequent requests reuse the same combined registry.

This design means:
- Built-in tools are never blocked by MCP connection latency.
- MCP tools appear atomically (all-or-nothing per server attempt).
- The `Tool::clone_box()` method added to the `Tool` trait enables this cloning.

### Retry on Reconnection

`close_mcp()` resets `mcp_connected` to `false` and clears sessions. The next call to `connect_mcp()` will attempt to re-establish all connections. This can be used to refresh MCP tool lists when a server is updated.

---

## 8. McpSession Lifetime

```rust
pub struct McpSession {
    _service: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    pub peer: Arc<rmcp::Peer<rmcp::RoleClient>>,
}
```

`McpSession` holds the `RunningService` that drives the background async task maintaining the HTTP connection.  **Dropping `McpSession` closes the connection.**

Sessions are stored in `mcp_sessions: Mutex<Vec<McpSession>>` inside `AgentLoop` and kept alive for as long as the agent runs. All `McpToolWrapper` instances for a given server share the same `peer` via `Arc<rmcp::Peer<…>>`.

---

## 9. Configuration Reference

See [Configuration System](config.md#mcpserverconfig) for the full `McpServerConfig` struct.

### Example

```json
{
  "tools": {
    "mcpServers": {
      "filesystem": {
        "url": "http://localhost:3000/mcp",
        "enabledTools": ["*"]
      },
      "remote-search": {
        "url": "https://api.example.com/mcp/sse",
        "type": "sse",
        "headers": {
          "Authorization": "Bearer sk-..."
        },
        "toolTimeout": 60,
        "enabledTools": ["web_search", "image_search"]
      }
    }
  }
}
```

---

## 10. Nanobot Comparison

| Aspect | nanobot (Python) | ombudsman (Rust) |
|---|---|---|
| MCP library | Python `mcp` SDK | Rust `rmcp` crate |
| Streamable HTTP | ✅ | ✅ |
| SSE transport | ✅ | ✅ (via StreamableHttpClientTransport) |
| stdio transport | ✅ | ❌ Not supported |
| Connection management | `AsyncExitStack` (context manager) | `McpSession` RAII struct |
| Lazy connection | No (eager at gateway start) | Yes (on first message) |
| Schema normalization | `_normalize_schema_for_openai()` | `normalize_schema_for_openai()` |
| Schema normalization logic | Functionally identical | Functionally identical |
| Tool namespacing | `mcp_{server}_{tool}` | `mcp_{server}_{tool}` ✅ |
| enabledTools filtering | ✅ | ✅ |
| Per-tool timeout | ✅ | ✅ |
| Retry on failure | No (skips server) | No (skips server, retries next message) |
| Custom headers | ✅ | ✅ |
