//! Native Anthropic Messages API provider.
//!
//! Converts the internal OpenAI-format messages to Anthropic's `/v1/messages`
//! format and back, so the rest of the codebase stays format-agnostic.

use crate::providers::base::{GenerationSettings, LLMProvider, LLMResponse, ToolCallRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{debug, warn};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
/// Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Thinking budget tokens by reasoning-effort level.
const THINKING_BUDGET_LOW: u32 = 2_000;
const THINKING_BUDGET_MEDIUM: u32 = 8_000;
const THINKING_BUDGET_HIGH: u32 = 16_000;

// ---------------------------------------------------------------------------
// Provider struct
// ---------------------------------------------------------------------------

/// Provider that calls the Anthropic Messages API directly.
pub struct AnthropicProvider {
    api_key: String,
    api_base: String,
    default_model: String,
    extra_headers: HashMap<String, String>,
    client: reqwest::Client,
    pub settings: GenerationSettings,
}

impl AnthropicProvider {
    pub fn new(
        api_key: &str,
        api_base: Option<&str>,
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
            api_base: api_base
                .unwrap_or(ANTHROPIC_API_BASE)
                .trim_end_matches('/')
                .to_string(),
            default_model: default_model.to_string(),
            extra_headers: extra_headers.unwrap_or_default(),
            client,
            settings: settings.unwrap_or_default(),
        }
    }

    // -----------------------------------------------------------------------
    // Message format conversion: OpenAI → Anthropic
    // -----------------------------------------------------------------------

    /// Convert a Vec of OpenAI-format messages into `(system, anthropic_messages)`.
    ///
    /// Rules:
    /// - `role=system` messages are extracted and joined as the `system` field.
    /// - `role=tool` messages (tool results) are collected and flushed as a
    ///   single `role=user` message with `tool_result` content blocks, matching
    ///   Anthropic's requirement that all tool results for one assistant turn sit
    ///   in a single user message.
    /// - `role=assistant` messages with `tool_calls` are converted to messages
    ///   with `tool_use` content blocks.
    /// - `role=user` messages flush any pending tool results first, then convert
    ///   image_url blocks to Anthropic base64 `image` blocks.
    pub(crate) fn convert_messages(messages: &[Value]) -> (Option<String>, Vec<Value>) {
        let mut system_parts: Vec<String> = Vec::new();
        let mut result: Vec<Value> = Vec::new();
        let mut pending_tool_results: Vec<Value> = Vec::new();

        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("");
            match role {
                "system" => {
                    if let Some(text) = msg["content"].as_str() {
                        system_parts.push(text.to_string());
                    }
                }
                "user" => {
                    flush_tool_results(&mut pending_tool_results, &mut result);
                    let content = convert_user_content(&msg["content"]);
                    result.push(json!({"role": "user", "content": content}));
                }
                "assistant" => {
                    flush_tool_results(&mut pending_tool_results, &mut result);
                    let content = convert_assistant_content(msg);
                    result.push(json!({"role": "assistant", "content": content}));
                }
                "tool" => {
                    let tool_use_id = msg["tool_call_id"].as_str().unwrap_or("").to_string();
                    let content_str = msg["content"].as_str().unwrap_or("").to_string();
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content_str,
                    }));
                }
                _ => {}
            }
        }

        flush_tool_results(&mut pending_tool_results, &mut result);

        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };

        (system, result)
    }

    // -----------------------------------------------------------------------
    // Tool format conversion: OpenAI → Anthropic
    // -----------------------------------------------------------------------

    /// Convert OpenAI-format tool definitions to Anthropic's `tools` array.
    ///
    /// OpenAI: `{"type": "function", "function": {"name": ..., "description": ..., "parameters": {...}}}`
    /// Anthropic: `{"name": ..., "description": ..., "input_schema": {...}}`
    pub(crate) fn convert_tools(tools: &[Value]) -> Vec<Value> {
        tools
            .iter()
            .filter_map(|tool| {
                let func = &tool["function"];
                let name = func["name"].as_str()?;
                let description = func["description"].as_str().unwrap_or("");
                let parameters = func["parameters"].clone();
                Some(json!({
                    "name": name,
                    "description": description,
                    "input_schema": parameters,
                }))
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Response parsing
    // -----------------------------------------------------------------------

    /// Extract `ToolCallRequest` values from Anthropic `tool_use` content blocks.
    fn parse_tool_calls(content: &[Value]) -> Vec<ToolCallRequest> {
        content
            .iter()
            .filter(|block| block["type"].as_str() == Some("tool_use"))
            .filter_map(|block| {
                let id = block["id"].as_str()?.to_string();
                let name = block["name"].as_str()?.to_string();
                let arguments: HashMap<String, Value> = if let Value::Object(map) = &block["input"]
                {
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                } else {
                    HashMap::new()
                };
                Some(ToolCallRequest { id, name, arguments })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Standalone helpers (module-private)
// ---------------------------------------------------------------------------

/// Flush accumulated tool results into the message list.
fn flush_tool_results(pending: &mut Vec<Value>, result: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    result.push(json!({"role": "user", "content": std::mem::take(pending)}));
}

/// Convert a user message `content` field (string or array) from OpenAI format
/// to the Anthropic content representation.
fn convert_user_content(content: &Value) -> Value {
    match content {
        Value::String(_) | Value::Null => content.clone(),
        Value::Array(blocks) => {
            let mut out: Vec<Value> = Vec::new();
            for block in blocks {
                let block_type = block["type"].as_str().unwrap_or("");
                match block_type {
                    "text" => {
                        out.push(json!({"type": "text", "text": block["text"]}));
                    }
                    "image_url" => {
                        if let Some(url) = block["image_url"]["url"].as_str() {
                            if let Some(img) = convert_image_url(url) {
                                out.push(img);
                            }
                        }
                    }
                    _ => {
                        // Pass through, but strip any internal _meta keys
                        let mut clean = block.clone();
                        if let Value::Object(ref mut map) = clean {
                            map.remove("_meta");
                        }
                        out.push(clean);
                    }
                }
            }
            if out.is_empty() {
                Value::String(String::new())
            } else {
                Value::Array(out)
            }
        }
        other => other.clone(),
    }
}

/// Convert an OpenAI-format `data:...;base64,...` URL or regular URL to an
/// Anthropic `image` content block.
fn convert_image_url(url: &str) -> Option<Value> {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some(semi) = rest.find(';') {
            let media_type = &rest[..semi];
            let after = &rest[semi + 1..];
            if let Some(comma) = after.find(',') {
                let encoding = &after[..comma];
                let data = &after[comma + 1..];
                if encoding == "base64" {
                    return Some(json!({
                        "type": "image",
                        "source": {"type": "base64", "media_type": media_type, "data": data},
                    }));
                }
            }
        }
    }
    // Regular HTTP/HTTPS URL → Anthropic "url" source type
    Some(json!({"type": "image", "source": {"type": "url", "url": url}}))
}

/// Convert an `assistant` OpenAI-format message (possibly with `tool_calls`) to
/// Anthropic content.  Returns either a `String` (text-only) or an `Array`
/// containing `text` and/or `tool_use` blocks.
fn convert_assistant_content(msg: &Value) -> Value {
    let mut blocks: Vec<Value> = Vec::new();

    // Text portion
    if let Some(text) = msg["content"].as_str() {
        if !text.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
    }

    // tool_calls → tool_use blocks
    if let Some(tool_calls) = msg["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_raw = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input: Value = serde_json::from_str(args_raw).unwrap_or(json!({}));
            blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
        }
    }

    match blocks.len() {
        0 => Value::String(String::new()),
        1 if blocks[0]["type"].as_str() == Some("text") => {
            Value::String(blocks[0]["text"].as_str().unwrap_or("").to_string())
        }
        _ => Value::Array(blocks),
    }
}

// ---------------------------------------------------------------------------
// LLMProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LLMProvider for AnthropicProvider {
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
        let model_name = model.unwrap_or(&self.default_model);
        let endpoint = format!("{}/messages", self.api_base);

        let (system, anthropic_messages) = Self::convert_messages(messages);

        let mut body = json!({
            "model": model_name,
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
        }

        // Temperature — omit when thinking mode is active
        if reasoning_effort.is_none() {
            body["temperature"] = json!(temperature);
        }

        // Extended thinking (reasoning_effort → budget_tokens)
        if let Some(effort) = reasoning_effort {
            let budget = match effort {
                "low" => THINKING_BUDGET_LOW,
                "high" => THINKING_BUDGET_HIGH,
                _ => THINKING_BUDGET_MEDIUM,
            };
            body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
        }

        // Tools
        if let Some(tool_defs) = tools {
            if !tool_defs.is_empty() {
                body["tools"] = json!(Self::convert_tools(tool_defs));
                let tc = tool_choice.and_then(|v| v.as_str()).unwrap_or("auto");
                body["tool_choice"] = json!({"type": match tc {
                    "required" => "any",
                    "none" => "none",
                    _ => "auto",
                }});
            }
        }

        debug!("Calling Anthropic: {} via {}", model_name, endpoint);

        let mut req = self
            .client
            .post(&endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body);

        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("Error calling Anthropic: {}", e);
                warn!("{}", msg);
                return LLMResponse::error(&msg);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let truncated = &body_text[..body_text.len().min(400)];
            let msg = format!("Anthropic API error {}: {}", status, truncated);
            warn!("{}", msg);
            return LLMResponse::error(&msg);
        }

        let resp_json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                return LLMResponse::error(&format!("Failed to parse Anthropic response: {}", e))
            }
        };

        let content_blocks = resp_json["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        // Concatenate text blocks
        let text_content: String = content_blocks
            .iter()
            .filter(|b| b["type"].as_str() == Some("text"))
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("");

        // Concatenate thinking blocks
        let thinking: Vec<&str> = content_blocks
            .iter()
            .filter(|b| b["type"].as_str() == Some("thinking"))
            .filter_map(|b| b["thinking"].as_str())
            .collect();
        let reasoning_content = if thinking.is_empty() {
            None
        } else {
            Some(thinking.join(""))
        };

        let tool_calls = Self::parse_tool_calls(&content_blocks);

        let finish_reason = match resp_json["stop_reason"].as_str().unwrap_or("end_turn") {
            "end_turn" => "stop",
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            other => other,
        }
        .to_string();

        let mut usage = HashMap::new();
        if let Some(u) = resp_json["usage"].as_object() {
            if let Some(it) = u.get("input_tokens").and_then(|v| v.as_u64()) {
                usage.insert("prompt_tokens".to_string(), it);
            }
            if let Some(ot) = u.get("output_tokens").and_then(|v| v.as_u64()) {
                usage.insert("completion_tokens".to_string(), ot);
            }
        }

        LLMResponse {
            content: if text_content.is_empty() {
                None
            } else {
                Some(text_content)
            },
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
    fn test_convert_simple_messages() {
        let messages = vec![
            json!({"role": "system", "content": "You are helpful."}),
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi there"}),
        ];
        let (system, msgs) = AnthropicProvider::convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("You are helpful."));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
    }

    #[test]
    fn test_convert_tool_calls_and_results() {
        let messages = vec![
            json!({"role": "user", "content": "Run a tool"}),
            json!({
                "role": "assistant",
                "content": "Using tool",
                "tool_calls": [
                    {
                        "id": "tc1",
                        "type": "function",
                        "function": {"name": "my_tool", "arguments": r#"{"x": 1}"#}
                    }
                ]
            }),
            json!({"role": "tool", "tool_call_id": "tc1", "name": "my_tool", "content": "result1"}),
            json!({"role": "tool", "tool_call_id": "tc2", "name": "other", "content": "result2"}),
        ];
        let (system, msgs) = AnthropicProvider::convert_messages(&messages);
        assert!(system.is_none());
        // user, assistant, user(tool_results merged)
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        // assistant content should have text + tool_use block
        let asst_content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(asst_content[0]["type"], "text");
        assert_eq!(asst_content[1]["type"], "tool_use");
        assert_eq!(asst_content[1]["id"], "tc1");
        // tool results merged into single user message
        assert_eq!(msgs[2]["role"], "user");
        let tool_content = msgs[2]["content"].as_array().unwrap();
        assert_eq!(tool_content.len(), 2);
        assert_eq!(tool_content[0]["type"], "tool_result");
        assert_eq!(tool_content[0]["tool_use_id"], "tc1");
        assert_eq!(tool_content[1]["type"], "tool_result");
        assert_eq!(tool_content[1]["tool_use_id"], "tc2");
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object", "properties": {}, "required": []}
            }
        })];
        let anthropic_tools = AnthropicProvider::convert_tools(&tools);
        assert_eq!(anthropic_tools.len(), 1);
        assert_eq!(anthropic_tools[0]["name"], "get_weather");
        assert_eq!(anthropic_tools[0]["description"], "Get weather");
        assert!(anthropic_tools[0]["input_schema"].is_object());
    }

    #[test]
    fn test_convert_image_url_data() {
        let img = convert_image_url("data:image/png;base64,abc123").unwrap();
        assert_eq!(img["type"], "image");
        assert_eq!(img["source"]["type"], "base64");
        assert_eq!(img["source"]["media_type"], "image/png");
        assert_eq!(img["source"]["data"], "abc123");
    }

    #[test]
    fn test_convert_image_url_http() {
        let img = convert_image_url("https://example.com/img.png").unwrap();
        assert_eq!(img["source"]["type"], "url");
        assert_eq!(img["source"]["url"], "https://example.com/img.png");
    }

    #[test]
    fn test_assistant_only_tool_calls_no_text() {
        // Assistant with no text content, only tool calls
        let msg = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"id": "id1", "type": "function", "function": {"name": "exec", "arguments": "{}"}}
            ]
        });
        let content = convert_assistant_content(&msg);
        // Should produce an array with just the tool_use block
        let arr = content.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "tool_use");
        assert_eq!(arr[0]["id"], "id1");
    }

    #[test]
    fn test_multiple_system_messages_joined() {
        let messages = vec![
            json!({"role": "system", "content": "Part 1"}),
            json!({"role": "system", "content": "Part 2"}),
            json!({"role": "user", "content": "Hi"}),
        ];
        let (system, _) = AnthropicProvider::convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("Part 1\n\nPart 2"));
    }
}
