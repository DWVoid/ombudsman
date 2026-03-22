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
    pub openai: ProviderConfig,
    pub anthropic: ProviderConfig,
    pub openrouter: ProviderConfig,
    pub deepseek: ProviderConfig,
    pub groq: ProviderConfig,
    pub gemini: ProviderConfig,
    pub moonshot: ProviderConfig,
    pub ollama: ProviderConfig,
    pub vllm: ProviderConfig,
    pub azure_openai: ProviderConfig,
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

/// MCP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
}

/// Tools configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsConfig {
    pub web_search: WebSearchConfig,
    pub exec: ExecToolConfig,
    pub web_proxy: Option<String>,
    pub restrict_to_workspace: bool,
    pub mcp_servers: Option<HashMap<String, McpServerConfig>>,
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
