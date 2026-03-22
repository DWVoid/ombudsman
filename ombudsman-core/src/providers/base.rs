//! Base LLM provider interface.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A tool call request from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: HashMap<String, Value>,
}

impl ToolCallRequest {
    /// Serialize to an OpenAI-style tool_call dict.
    pub fn to_openai_tool_call(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "type": "function",
            "function": {
                "name": self.name,
                "arguments": serde_json::to_string(&self.arguments).unwrap_or_default(),
            }
        })
    }
}

/// Response from an LLM provider.
#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub finish_reason: String,
    pub usage: HashMap<String, u64>,
    pub reasoning_content: Option<String>,
}

impl LLMResponse {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    pub fn error(msg: &str) -> Self {
        Self {
            content: Some(msg.to_string()),
            tool_calls: Vec::new(),
            finish_reason: "error".to_string(),
            usage: HashMap::new(),
            reasoning_content: None,
        }
    }
}

/// Default generation settings.
#[derive(Debug, Clone)]
pub struct GenerationSettings {
    pub temperature: f64,
    pub max_tokens: u32,
    pub reasoning_effort: Option<String>,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            max_tokens: 4096,
            reasoning_effort: None,
        }
    }
}

/// Transient error markers for retry logic.
const TRANSIENT_ERROR_MARKERS: &[&str] = &[
    "429",
    "rate limit",
    "500",
    "502",
    "503",
    "504",
    "overloaded",
    "timeout",
    "timed out",
    "connection",
    "server error",
    "temporarily unavailable",
];

/// Retry delays in seconds.
const RETRY_DELAYS: &[u64] = &[1, 2, 4];

/// Check if an error message indicates a transient failure.
pub fn is_transient_error(content: Option<&str>) -> bool {
    let err = content.unwrap_or("").to_lowercase();
    TRANSIENT_ERROR_MARKERS.iter().any(|m| err.contains(m))
}

/// Abstract LLM provider trait.
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Send a chat completion request.
    async fn chat(
        &self,
        messages: &[Value],
        tools: Option<&[Value]>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f64,
        reasoning_effort: Option<&str>,
        tool_choice: Option<&Value>,
    ) -> LLMResponse;

    /// Get the default model for this provider.
    fn get_default_model(&self) -> &str;

    /// Get generation settings.
    fn generation_settings(&self) -> GenerationSettings {
        GenerationSettings::default()
    }

    /// Call chat with automatic retry on transient errors.
    async fn chat_with_retry(
        &self,
        messages: &[Value],
        tools: Option<&[Value]>,
        model: Option<&str>,
        max_tokens: Option<u32>,
        temperature: Option<f64>,
        reasoning_effort: Option<&str>,
        tool_choice: Option<&Value>,
    ) -> LLMResponse {
        let settings = self.generation_settings();
        let max_tokens = max_tokens.unwrap_or(settings.max_tokens);
        let temperature = temperature.unwrap_or(settings.temperature);

        for &delay in RETRY_DELAYS {
            let response = self.chat(
                messages,
                tools,
                model,
                max_tokens,
                temperature,
                reasoning_effort,
                tool_choice,
            ).await;

            if response.finish_reason != "error" {
                return response;
            }

            if !is_transient_error(response.content.as_deref()) {
                return response;
            }

            tracing::warn!(
                "Transient LLM error (retrying in {}s): {}",
                delay,
                response.content.as_deref().unwrap_or("")
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
        }

        // Final attempt
        self.chat(
            messages,
            tools,
            model,
            max_tokens,
            temperature,
            reasoning_effort,
            tool_choice,
        ).await
    }
}
