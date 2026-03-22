//! OpenAI-compatible HTTP API provider.
//! Supports OpenAI, OpenRouter, DeepSeek, Groq, Gemini (via OpenAI-compat endpoint),
//! Moonshot, MiniMax, Ollama, vLLM, and any other OpenAI-compatible endpoint.

use crate::providers::base::{GenerationSettings, LLMProvider, LLMResponse, ToolCallRequest};
use async_trait::async_trait;
use rand::Rng;
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{debug, warn};

/// OpenAI-compatible provider.
pub struct OpenAIProvider {
    api_key: String,
    api_base: String,
    default_model: String,
    extra_headers: HashMap<String, String>,
    client: reqwest::Client,
    pub settings: GenerationSettings,
}

impl OpenAIProvider {
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
        Self {
            api_key: api_key.to_string(),
            api_base: api_base.trim_end_matches('/').to_string(),
            default_model: default_model.to_string(),
            extra_headers: extra_headers.unwrap_or_default(),
            client,
            settings: settings.unwrap_or_default(),
        }
    }

    /// Fallback tool call ID when the API response omits one.
    fn short_tool_id() -> String {
        let mut rng = rand::thread_rng();
        (0..9)
            .map(|_| {
                let idx = rng.gen_range(0..36u8);
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'a' + idx - 10) as char
                }
            })
            .collect()
    }

    /// Returns `true` when `temperature` should be omitted for this model.
    ///
    /// OpenAI o-series reasoning models reject the `temperature` parameter.
    pub(crate) fn omit_temperature(model: &str) -> bool {
        let lower = model.to_lowercase();
        // o1 / o3 / o4 families
        if lower.starts_with("o1") || lower.starts_with("o3") || lower.starts_with("o4") {
            return true;
        }
        false
    }

    /// Sanitize messages before sending: strip internal `_meta` / `reasoning_content`
    /// fields that are not part of the OpenAI wire format, and fix `None`/empty
    /// content on assistant messages that also carry tool_calls.
    fn sanitize_messages(messages: &[Value]) -> Vec<Value> {
        const STRIP_KEYS: &[&str] = &["reasoning_content", "_meta"];
        messages
            .iter()
            .map(|msg| {
                let mut clean = msg.clone();
                if let Value::Object(ref mut map) = clean {
                    for key in STRIP_KEYS {
                        map.remove(*key);
                    }
                    // Ensure assistant+tool_calls messages have content=null not missing
                    if map.get("role").and_then(|r| r.as_str()) == Some("assistant")
                        && map.contains_key("tool_calls")
                        && !map.contains_key("content")
                    {
                        map.insert("content".to_string(), Value::Null);
                    }
                    // Replace genuinely empty string content with null for assistant messages
                    if map.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                        if map
                            .get("content")
                            .and_then(|c| c.as_str())
                            .map(|s| s.is_empty())
                            .unwrap_or(false)
                        {
                            map.insert("content".to_string(), Value::Null);
                        }
                    }
                }
                clean
            })
            .collect()
    }

    /// Parse tool calls from the response, preserving API-provided IDs.
    fn parse_tool_calls(raw_tool_calls: &[Value]) -> Vec<ToolCallRequest> {
        raw_tool_calls
            .iter()
            .filter_map(|tc| {
                let name = tc["function"]["name"].as_str()?.to_string();
                let args_raw = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: HashMap<String, Value> =
                    serde_json::from_str(args_raw).unwrap_or_default();
                // Use the ID from the API response; generate a fallback only if missing.
                let id = tc["id"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(Self::short_tool_id);
                Some(ToolCallRequest { id, name, arguments })
            })
            .collect()
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
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
        let model = model.unwrap_or(&self.default_model);
        let endpoint = format!("{}/chat/completions", self.api_base);

        let clean_messages = Self::sanitize_messages(messages);

        let mut body = json!({
            "model": model,
            "messages": clean_messages,
            "max_tokens": max_tokens,
        });

        // Omit temperature for reasoning models (o1/o3/o4)
        if !Self::omit_temperature(model) {
            body["temperature"] = json!(temperature);
        }

        // reasoning_effort for o-series models
        if let Some(effort) = reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }

        if let Some(tools) = tools {
            if !tools.is_empty() {
                body["tools"] = json!(tools);
                body["tool_choice"] = tool_choice.cloned().unwrap_or(json!("auto"));
            }
        }

        debug!("Calling LLM: {} via {}", model, endpoint);

        let mut req = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body);

        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("Error calling LLM: {}", e);
                warn!("{}", msg);
                return LLMResponse::error(&msg);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let truncated = &body_text[..body_text.len().min(400)];
            let msg = format!("LLM API error {}: {}", status, truncated);
            warn!("{}", msg);
            return LLMResponse::error(&msg);
        }

        let resp_json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                return LLMResponse::error(&format!("Failed to parse LLM response: {}", e));
            }
        };

        let choices = resp_json["choices"].as_array();
        let choice = choices.and_then(|c| c.first());

        let (content, tool_calls, finish_reason, reasoning_content) = if let Some(choice) = choice {
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
            // Some providers (DeepSeek-R1, Qwen, etc.) return reasoning in this field
            let reasoning_content = msg["reasoning_content"].as_str().map(|s| s.to_string());
            (content, tool_calls, finish_reason, reasoning_content)
        } else {
            (None, Vec::new(), "stop".to_string(), None)
        };

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
    use serde_json::json;

    #[test]
    fn test_omit_temperature_reasoning_models() {
        assert!(OpenAIProvider::omit_temperature("o1-mini"));
        assert!(OpenAIProvider::omit_temperature("o3-pro"));
        assert!(OpenAIProvider::omit_temperature("o4-preview"));
        assert!(!OpenAIProvider::omit_temperature("gpt-4o"));
        assert!(!OpenAIProvider::omit_temperature("gpt-4-turbo"));
        assert!(!OpenAIProvider::omit_temperature("claude-opus-4-5"));
    }

    #[test]
    fn test_sanitize_strips_internal_keys() {
        let messages = vec![json!({
            "role": "assistant",
            "content": "hello",
            "reasoning_content": "private",
        })];
        let clean = OpenAIProvider::sanitize_messages(&messages);
        assert_eq!(clean[0]["content"], "hello");
        assert!(clean[0].get("reasoning_content").is_none());
    }

    #[test]
    fn test_sanitize_assistant_empty_content_with_tool_calls() {
        let messages = vec![json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"id": "1", "type": "function", "function": {"name": "x", "arguments": "{}"}}],
        })];
        let clean = OpenAIProvider::sanitize_messages(&messages);
        // empty string content → null when tool_calls present
        assert!(clean[0]["content"].is_null());
    }
}


/// OpenAI-compatible provider.
pub struct OpenAIProvider {
    api_key: String,
    api_base: String,
    default_model: String,
    extra_headers: HashMap<String, String>,
    client: reqwest::Client,
    pub settings: GenerationSettings,
}

impl OpenAIProvider {
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
        Self {
            api_key: api_key.to_string(),
            api_base: api_base.trim_end_matches('/').to_string(),
            default_model: default_model.to_string(),
            extra_headers: extra_headers.unwrap_or_default(),
            client,
            settings: settings.unwrap_or_default(),
        }
    }

    /// Generate a short random alphanumeric tool call ID.
    fn short_tool_id() -> String {
        let mut rng = rand::thread_rng();
        (0..9)
            .map(|_| {
                let idx = rng.gen_range(0..36);
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'a' + idx - 10) as char
                }
            })
            .collect()
    }

    /// Parse tool calls from the response.
    fn parse_tool_calls(raw_tool_calls: &[Value]) -> Vec<ToolCallRequest> {
        raw_tool_calls
            .iter()
            .filter_map(|tc| {
                let name = tc["function"]["name"].as_str()?.to_string();
                let args_raw = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: HashMap<String, Value> =
                    serde_json::from_str(args_raw).unwrap_or_default();
                Some(ToolCallRequest {
                    id: Self::short_tool_id(),
                    name,
                    arguments,
                })
            })
            .collect()
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: &[Value],
        tools: Option<&[Value]>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f64,
        _reasoning_effort: Option<&str>,
        tool_choice: Option<&Value>,
    ) -> LLMResponse {
        let model = model.unwrap_or(&self.default_model);
        let endpoint = format!("{}/chat/completions", self.api_base);

        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(tools) = tools {
            if !tools.is_empty() {
                body["tools"] = json!(tools);
                body["tool_choice"] = tool_choice.cloned().unwrap_or(json!("auto"));
            }
        }

        debug!("Calling LLM: {} via {}", model, endpoint);

        let mut req = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body);

        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("Error calling LLM: {}", e);
                warn!("{}", msg);
                return LLMResponse::error(&msg);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let msg = format!("LLM API error {}: {}", status, &body_text[..body_text.len().min(200)]);
            warn!("{}", msg);
            return LLMResponse::error(&msg);
        }

        let resp_json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                return LLMResponse::error(&format!("Failed to parse LLM response: {}", e));
            }
        };

        // Extract content and tool calls from the first choice
        let choices = resp_json["choices"].as_array();
        let choice = choices.and_then(|c| c.first());

        let (content, tool_calls, finish_reason) = if let Some(choice) = choice {
            let msg = &choice["message"];
            let content = msg["content"].as_str().map(|s| s.to_string());
            let finish_reason = choice["finish_reason"]
                .as_str()
                .unwrap_or("stop")
                .to_string();

            let raw_tcs = msg["tool_calls"].as_array().map(|v| v.as_slice()).unwrap_or(&[]);
            let tool_calls = Self::parse_tool_calls(raw_tcs);

            (content, tool_calls, finish_reason)
        } else {
            (None, Vec::new(), "stop".to_string())
        };

        // Extract usage
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
            reasoning_content: None,
        }
    }

    fn get_default_model(&self) -> &str {
        &self.default_model
    }

    fn generation_settings(&self) -> GenerationSettings {
        self.settings.clone()
    }
}
