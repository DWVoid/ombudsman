//! Configuration schema for ombudsman.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent default configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentDefaults {
    pub workspace: String,
    pub model: String,
    pub provider: String,
    pub max_tokens: u32,
    pub context_window_tokens: u32,
    pub temperature: f64,
    pub max_tool_iterations: u32,
    pub reasoning_effort: Option<String>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: "~/.ombudsman/workspace".to_string(),
            model: "gpt-4o".to_string(),
            provider: "auto".to_string(),
            max_tokens: 8192,
            context_window_tokens: 65_536,
            temperature: 0.1,
            max_tool_iterations: 40,
            reasoning_effort: None,
        }
    }
}

/// Agent configuration container.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
}

/// Individual LLM provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub api_key: String,
    pub api_base: Option<String>,
    pub extra_headers: Option<HashMap<String, String>>,
}

/// Configuration for all LLM providers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProvidersConfig {
    /// OpenAI — api.openai.com
    pub openai: ProviderConfig,
    /// Anthropic — api.anthropic.com (native Messages API)
    pub anthropic: ProviderConfig,
    /// OpenRouter — openrouter.ai (gateway; routes any model)
    pub openrouter: ProviderConfig,
    /// DeepSeek — api.deepseek.com
    pub deepseek: ProviderConfig,
    /// Groq — api.groq.com
    pub groq: ProviderConfig,
    /// Google Gemini — via OpenAI-compat endpoint
    pub gemini: ProviderConfig,
    /// Moonshot / Kimi — api.moonshot.ai
    pub moonshot: ProviderConfig,
    /// MiniMax — api.minimax.io
    pub minimax: ProviderConfig,
    /// Ollama — local models via OpenAI-compat endpoint
    pub ollama: ProviderConfig,
    /// vLLM / any OpenAI-compatible local server
    pub vllm: ProviderConfig,
    /// Azure OpenAI — requires api_base; uses `api-key` header
    pub azure_openai: ProviderConfig,
    /// AiHubMix — aihubmix.com OpenAI-compatible gateway
    pub aihubmix: ProviderConfig,
    /// SiliconFlow — api.siliconflow.cn OpenAI-compatible gateway
    pub siliconflow: ProviderConfig,
    /// VolcEngine (火山引擎) — ark.cn-beijing.volces.com
    pub volcengine: ProviderConfig,
    /// Custom — any OpenAI-compatible endpoint (fallback)
    pub custom: ProviderConfig,
}

/// Web search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSearchConfig {
    pub provider: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_results: u32,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: "duckduckgo".to_string(),
            api_key: None,
            base_url: None,
            max_results: 5,
        }
    }
}

/// Shell/exec tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecToolConfig {
    pub enable: bool,
    pub timeout: u32,
    pub path_append: String,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self {
            enable: true,
            timeout: 60,
            path_append: String::new(),
        }
    }
}

/// MCP server transport type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpTransportType {
    /// Legacy Server-Sent Events transport (HTTP SSE endpoint).
    #[serde(rename = "sse")]
    Sse,
    /// New MCP Streamable HTTP transport.
    #[serde(rename = "streamableHttp")]
    StreamableHttp,
}

/// MCP server configuration (HTTP transports only — stdio is not supported).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpServerConfig {
    /// Transport type; auto-detected from URL if omitted.
    /// Accepts "sse" or "streamableHttp".
    /// stdio is not supported.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub transport_type: Option<McpTransportType>,
    /// HTTP/SSE endpoint URL.
    pub url: String,
    /// Custom HTTP headers to include with every request.
    pub headers: HashMap<String, String>,
    /// Seconds before a tool call is cancelled (default: 30).
    pub tool_timeout: u32,
    /// Which tools to register.
    /// Accepts raw MCP tool names or wrapped `mcp_<server>_<tool>` names.
    /// `["*"]` (default) = all tools; `[]` = no tools.
    pub enabled_tools: Vec<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            transport_type: None,
            url: String::new(),
            headers: HashMap::new(),
            tool_timeout: 30,
            enabled_tools: vec!["*".to_string()],
        }
    }
}

/// Tools configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsConfig {
    pub web_search: WebSearchConfig,
    pub exec: ExecToolConfig,
    pub web_proxy: Option<String>,
    pub restrict_to_workspace: bool,
    /// MCP servers to connect to. Keys are server names.
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

/// Channels configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelsConfig {
    pub send_progress: bool,
    pub send_tool_hints: bool,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            send_progress: true,
            send_tool_hints: false,
            extra: HashMap::new(),
        }
    }
}

/// Root configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub agents: AgentsConfig,
    pub providers: ProvidersConfig,
    pub tools: ToolsConfig,
    pub channels: ChannelsConfig,
}
