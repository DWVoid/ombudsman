# Configuration System

**Source:** `src/config/`

The configuration system provides a typed, JSON-based configuration schema that controls every tunable aspect of ombudsman's runtime behaviour.

---

## Table of Contents

1. [File Location and Paths](#1-file-location-and-paths)
2. [Configuration Schema](#2-configuration-schema)
3. [Default Values](#3-default-values)
4. [Loading and Saving](#4-loading-and-saving)

---

## 1. File Location and Paths

**File:** `src/config/paths.rs`

All ombudsman state is stored under a single base directory:

| Path | Description |
|---|---|
| `~/.ombudsman/` | Base directory |
| `~/.ombudsman/config.json` | Runtime configuration file |
| `~/.ombudsman/workspace/` | Default workspace directory |

Tilde (`~`) expansion is handled using the `dirs` crate to locate the user's home directory on all platforms.

The `ConfigPaths` struct exposes:

```rust
pub fn base_dir() -> PathBuf
pub fn config_file() -> PathBuf
pub fn default_workspace() -> PathBuf
```

---

## 2. Configuration Schema

**File:** `src/config/schema.rs`

All structs derive `serde::Serialize`, `serde::Deserialize`, and `Default`.  JSON keys use `camelCase` (via `#[serde(rename_all = "camelCase")]`).

### Root Config

```rust
pub struct Config {
    pub agents:    AgentsConfig,
    pub providers: ProvidersConfig,
    pub tools:     ToolsConfig,
    pub channels:  ChannelsConfig,
}
```

### AgentsConfig / AgentDefaults

```rust
pub struct AgentDefaults {
    pub workspace:             String,        // path to workspace directory
    pub model:                 String,        // LLM model name
    pub provider:              String,        // provider name or "auto"
    pub max_tokens:            u32,           // max output tokens per LLM call
    pub context_window_tokens: u32,           // context window budget for history trimming
    pub temperature:           f64,           // sampling temperature
    pub max_tool_iterations:   u32,           // max agent loop iterations
    pub reasoning_effort:      Option<String>,// for reasoning-mode models
}
```

### ProvidersConfig / ProviderConfig

```rust
pub struct ProvidersConfig {
    pub openai:       ProviderConfig,
    pub anthropic:    ProviderConfig,
    pub openrouter:   ProviderConfig,
    pub deepseek:     ProviderConfig,
    pub groq:         ProviderConfig,
    pub gemini:       ProviderConfig,
    pub moonshot:     ProviderConfig,
    pub minimax:      ProviderConfig,
    pub ollama:       ProviderConfig,
    pub vllm:         ProviderConfig,
    pub azure_openai: ProviderConfig,
    pub aihubmix:     ProviderConfig,
    pub siliconflow:  ProviderConfig,
    pub volcengine:   ProviderConfig,
    pub custom:       ProviderConfig,
}

pub struct ProviderConfig {
    pub api_key:       String,
    pub api_base:      Option<String>,
    pub extra_headers: Option<HashMap<String, String>>,
}
```

Each provider field is optional in the JSON; a missing provider block defaults to an empty `ProviderConfig`.

### ToolsConfig

```rust
pub struct ToolsConfig {
    pub web_search:            WebSearchConfig,
    pub exec:                  ExecToolConfig,
    pub web_proxy:             Option<String>,
    pub restrict_to_workspace: bool,
    pub mcp_servers:           HashMap<String, McpServerConfig>,
}
```

#### WebSearchConfig

```rust
pub struct WebSearchConfig {
    pub provider:    String,        // "brave" or "duckduckgo"
    pub api_key:     Option<String>,
    pub base_url:    Option<String>,
    pub max_results: u32,           // default 5
}
```

#### ExecToolConfig

```rust
pub struct ExecToolConfig {
    pub enable:      bool,    // enable/disable the exec tool
    pub timeout:     u32,     // default command timeout in seconds
    pub path_append: String,  // prepended to PATH for exec commands
}
```

#### McpTransportType

```rust
pub enum McpTransportType {
    Sse,            // serialised as "sse"
    StreamableHttp, // serialised as "streamableHttp"
}
```

#### McpServerConfig

```rust
pub struct McpServerConfig {
    pub transport_type: Option<McpTransportType>, // JSON field: "type"
    pub url:            String,
    pub headers:        HashMap<String, String>,
    pub tool_timeout:   u32,               // default 30 seconds
    pub enabled_tools:  Vec<String>,       // default ["*"] = all tools
}
```

| Field | Description |
|---|---|
| `type` | Transport protocol; auto-detected from URL if omitted |
| `url` | HTTP/SSE endpoint URL. Required; entries without a URL are skipped |
| `headers` | Custom HTTP headers sent with every request (e.g. `Authorization`) |
| `toolTimeout` | Per-tool-call timeout in seconds (default 30) |
| `enabledTools` | Whitelist of tool names to register; `["*"]` registers all |

See [MCP Subsystem](mcp.md) for full runtime semantics.

### ChannelsConfig

```rust
pub struct ChannelsConfig {
    pub send_progress:   bool,
    pub send_tool_hints: bool,
    // Additional channel-specific keys are captured here via #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
```

---

## 3. Default Values

When a config key is absent from `config.json`, the following defaults apply:

| Key | Default |
|---|---|
| `agents.defaults.workspace` | `~/.ombudsman/workspace` |
| `agents.defaults.model` | `gpt-4o` |
| `agents.defaults.provider` | `auto` |
| `agents.defaults.maxTokens` | `8192` |
| `agents.defaults.contextWindowTokens` | `65536` |
| `agents.defaults.temperature` | `0.1` |
| `agents.defaults.maxToolIterations` | `40` |
| `tools.webSearch.provider` | `duckduckgo` |
| `tools.webSearch.maxResults` | `5` |
| `tools.exec.enable` | `true` |
| `tools.exec.timeout` | `60` |
| `tools.restrictToWorkspace` | `false` |
| `tools.mcpServers` | `{}` (empty — no MCP servers) |
| `channels.sendProgress` | `true` |
| `channels.sendToolHints` | `false` |

---

## 4. Loading and Saving

**File:** `src/config/loader.rs`

### Load

```rust
pub fn load_config(path: Option<&Path>) -> anyhow::Result<Config>
```

1. Determines the config file path: the provided `path`, or `~/.ombudsman/config.json`.
2. If the file does not exist, returns `Config::default()`.
3. Reads the file and deserialises it with `serde_json`.
4. Missing keys silently adopt their `Default` values.

### Save

```rust
pub fn save_config(config: &Config, path: Option<&Path>) -> anyhow::Result<()>
```

1. Creates the parent directory if it does not exist.
2. Serialises `config` to pretty-printed JSON.
3. Writes atomically to the target path.

### Typical Minimal Config

```json
{
  "providers": {
    "openai": {
      "apiKey": "sk-…"
    }
  }
}
```

All other values use their defaults. Only the sections you want to override need to be present.
