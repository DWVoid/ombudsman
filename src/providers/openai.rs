//! OpenAI-compatible HTTP API provider.
//! Supports OpenAI, Anthropic (via compat layer), OpenRouter, and any
//! OpenAI-compatible endpoint.

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
