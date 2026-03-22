//! Azure OpenAI provider.
//!
//! Differences from vanilla OpenAI:
//! - URL:  `{api_base}/openai/deployments/{deployment}/chat/completions?api-version=2024-10-21`
//! - Auth: `api-key: {key}` header (not `Authorization: Bearer`)
//! - Body: same JSON shape except `max_completion_tokens` replaces `max_tokens`
//!         for models that no longer support `max_tokens`.
//! - Temperature is omitted for reasoning models (gpt-5*, o1*, o3*, o4* prefixes).

use crate::providers::base::{GenerationSettings, LLMProvider, LLMResponse, ToolCallRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{debug, warn};

const API_VERSION: &str = "2024-10-21";

/// Azure OpenAI-specific message keys (strips provider-private fields).
const AZURE_MSG_KEYS: &[&str] = &[
    "role",
    "content",
    "tool_calls",
    "tool_call_id",
    "name",
    "function_call",
];

// ---------------------------------------------------------------------------
// Provider struct
// ---------------------------------------------------------------------------

/// Provider that calls the Azure OpenAI `chat/completions` endpoint.
pub struct AzureOpenAIProvider {
    api_key: String,
    api_base: String,
    default_model: String,
    extra_headers: HashMap<String, String>,
    client: reqwest::Client,
    pub settings: GenerationSettings,
}

impl AzureOpenAIProvider {
    pub fn new(
        api_key: &str,
        api_base: &str,
        default_model: &str,
        extra_headers: Option<HashMap<String, String>>,
        settings: Option<GenerationSettings>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");
        let api_base = api_base.trim_end_matches('/').to_string();
        Self {
            api_key: api_key.to_string(),
            api_base,
            default_model: default_model.to_string(),
            extra_headers: extra_headers.unwrap_or_default(),
            client,
            settings: settings.unwrap_or_default(),
        }
    }

    /// Build the Azure chat-completions URL for a given deployment name.
    ///
    /// ```text
    /// {api_base}/openai/deployments/{deployment}/chat/completions?api-version=2024-10-21
    /// ```
    pub(crate) fn build_url(&self, deployment: &str) -> String {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.api_base, deployment, API_VERSION,
        )
    }

    /// Return `true` when `temperature` should be omitted for the deployment.
    ///
    /// Reasoning models (gpt-5*, o1*, o3*, o4*) reject the `temperature` param.
    pub(crate) fn omit_temperature(deployment: &str, reasoning_effort: Option<&str>) -> bool {
        if reasoning_effort.is_some() {
            return true;
        }
        let lower = deployment.to_lowercase();
        lower.starts_with("gpt-5")
            || lower.starts_with("o1")
            || lower.starts_with("o3")
            || lower.starts_with("o4")
    }

    /// Strip provider-internal keys from messages (e.g. `reasoning_content`).
    fn sanitize_messages(messages: &[Value]) -> Vec<Value> {
        messages
            .iter()
            .map(|msg| {
                let mut clean = serde_json::Map::new();
                if let Value::Object(map) = msg {
                    for key in AZURE_MSG_KEYS {
                        if let Some(v) = map.get(*key) {
                            clean.insert(key.to_string(), v.clone());
                        }
                    }
                    // Normalise assistant with no content
                    if clean.get("role").and_then(|r| r.as_str()) == Some("assistant")
                        && !clean.contains_key("content")
                    {
                        clean.insert("content".to_string(), Value::Null);
                    }
                }
                Value::Object(clean)
            })
            .collect()
    }

    /// Parse tool calls from an OpenAI-format `message` object.
    fn parse_tool_calls(raw_tool_calls: &[Value]) -> Vec<ToolCallRequest> {
        raw_tool_calls
            .iter()
            .filter_map(|tc| {
                let id = tc["id"].as_str()?.to_string();
                let name = tc["function"]["name"].as_str()?.to_string();
                let args_raw = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: HashMap<String, Value> =
                    serde_json::from_str(args_raw).unwrap_or_default();
                Some(ToolCallRequest { id, name, arguments })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// LLMProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LLMProvider for AzureOpenAIProvider {
    async fn chat(
        &self,
        messages: &[Value],
        tools: Option<&[Value]>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f64,
        reasoning_effort: Option<&str>,
        tool_choice: Option<&Value>,
    ) -> LLMResponse {
        let deployment = model.unwrap_or(&self.default_model);
        let endpoint = self.build_url(deployment);

        let clean_messages = Self::sanitize_messages(messages);

        let mut body = json!({
            "messages": clean_messages,
            // Azure 2024-10-21 uses max_completion_tokens
            "max_completion_tokens": max_tokens,
        });

        if !Self::omit_temperature(deployment, reasoning_effort) {
            body["temperature"] = json!(temperature);
        }

        if let Some(effort) = reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }

        if let Some(tool_defs) = tools {
            if !tool_defs.is_empty() {
                body["tools"] = json!(tool_defs);
                body["tool_choice"] = tool_choice.cloned().unwrap_or(json!("auto"));
            }
        }

        debug!("Calling Azure OpenAI: {} via {}", deployment, endpoint);

        let mut req = self
            .client
            .post(&endpoint)
            .header("api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body);

        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("Error calling Azure OpenAI: {}", e);
                warn!("{}", msg);
                return LLMResponse::error(&msg);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let truncated = &body_text[..body_text.len().min(400)];
            let msg = format!("Azure OpenAI API error {}: {}", status, truncated);
            warn!("{}", msg);
            return LLMResponse::error(&msg);
        }

        let resp_json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                return LLMResponse::error(&format!(
                    "Failed to parse Azure OpenAI response: {}",
                    e
                ))
            }
        };

        let choices = resp_json["choices"].as_array();
        let choice = choices.and_then(|c| c.first());

        let (content, tool_calls, finish_reason) = if let Some(choice) = choice {
            let msg = &choice["message"];
            let content = msg["content"].as_str().map(|s| s.to_string());
            let finish_reason = choice["finish_reason"]
                .as_str()
                .unwrap_or("stop")
                .to_string();
            let raw_tcs = msg["tool_calls"]
                .as_array()
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let tool_calls = Self::parse_tool_calls(raw_tcs);
            (content, tool_calls, finish_reason)
        } else {
            (None, Vec::new(), "stop".to_string())
        };

        let reasoning_content = resp_json["choices"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["message"]["reasoning_content"].as_str())
            .map(|s| s.to_string());

        let mut usage = HashMap::new();
        if let Some(u) = resp_json["usage"].as_object() {
            if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                usage.insert("prompt_tokens".to_string(), pt);
            }
            if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                usage.insert("completion_tokens".to_string(), ct);
            }
        }

        LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
            reasoning_content,
        }
    }

    fn get_default_model(&self) -> &str {
        &self.default_model
    }

    fn generation_settings(&self) -> GenerationSettings {
        self.settings.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> AzureOpenAIProvider {
        AzureOpenAIProvider::new(
            "key",
            "https://my-resource.openai.azure.com",
            "gpt-4o",
            None,
            None,
        )
    }

    #[test]
    fn test_build_url() {
        let p = provider();
        let url = p.build_url("my-deploy");
        assert!(url.starts_with("https://my-resource.openai.azure.com/openai/deployments/my-deploy/chat/completions"));
        assert!(url.contains("api-version=2024-10-21"));
    }

    #[test]
    fn test_omit_temperature_reasoning_models() {
        assert!(AzureOpenAIProvider::omit_temperature("o1-mini", None));
        assert!(AzureOpenAIProvider::omit_temperature("o3-pro", None));
        assert!(AzureOpenAIProvider::omit_temperature("o4-preview", None));
        assert!(AzureOpenAIProvider::omit_temperature("gpt-5.2-chat", None));
        assert!(!AzureOpenAIProvider::omit_temperature("gpt-4o", None));
        assert!(!AzureOpenAIProvider::omit_temperature("gpt-4-turbo", None));
    }

    #[test]
    fn test_omit_temperature_with_reasoning_effort() {
        // Regardless of model, reasoning_effort suppresses temperature
        assert!(AzureOpenAIProvider::omit_temperature("gpt-4o", Some("medium")));
    }

    #[test]
    fn test_sanitize_strips_internal_keys() {
        use serde_json::json;
        let messages = vec![json!({
            "role": "assistant",
            "content": "hello",
            "reasoning_content": "private",
            "some_other_key": 123,
        })];
        let clean = AzureOpenAIProvider::sanitize_messages(&messages);
        assert_eq!(clean[0]["role"], "assistant");
        assert_eq!(clean[0]["content"], "hello");
        assert!(clean[0].get("reasoning_content").is_none());
        assert!(clean[0].get("some_other_key").is_none());
    }
}
