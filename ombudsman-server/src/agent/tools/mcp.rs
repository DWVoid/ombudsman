//! MCP client: connects to MCP HTTP servers and wraps their tools as native ombudsman tools.
//!
//! Only HTTP transports are supported (SSE and Streamable HTTP).
//! stdio is explicitly NOT supported.

use crate::agent::tools::base::Tool;
use crate::agent::tools::registry::ToolRegistry;
use crate::config::schema::{McpServerConfig, McpTransportType};
use async_trait::async_trait;
use http::{HeaderName, HeaderValue};
use rmcp::model::CallToolRequestParams;
use rmcp::service::ServiceExt;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Normalize JSON Schema nullable patterns so OpenAI-style tool definitions accept them.
///
/// Handles:
/// - `type: ["T", "null"]` → `type: "T", nullable: true`
/// - `anyOf: [{type: "T"}, {type: "null"}]` → unwrapped `type: "T", nullable: true`
/// - Recursively normalizes `properties` and `items`
pub fn normalize_schema_for_openai(schema: Value) -> Value {
    let obj = match schema {
        Value::Object(m) => m,
        other => return other,
    };

    let mut normalized = obj;

    // Handle `type: ["T", "null"]`
    if let Some(Value::Array(types)) = normalized.get("type").cloned() {
        let non_null: Vec<&Value> = types.iter().filter(|t| t.as_str() != Some("null")).collect();
        let has_null = types.iter().any(|t| t.as_str() == Some("null"));
        if has_null && non_null.len() == 1 {
            normalized.insert("type".to_string(), non_null[0].clone());
            normalized.insert("nullable".to_string(), Value::Bool(true));
        }
    }

    // Handle `anyOf` / `oneOf` with a single non-null branch
    'outer: for key in &["anyOf", "oneOf"] {
        if let Some(Value::Array(options)) = normalized.get(*key).cloned() {
            let mut non_null_branches: Vec<Map<String, Value>> = Vec::new();
            let mut saw_null = false;

            for option in &options {
                if let Value::Object(obj) = option {
                    if obj.get("type").and_then(|v| v.as_str()) == Some("null") {
                        saw_null = true;
                    } else {
                        non_null_branches.push(obj.clone());
                    }
                } else {
                    // Not all options are objects — skip normalization for this key
                    continue 'outer;
                }
            }

            if saw_null && non_null_branches.len() == 1 {
                // Remove the oneOf/anyOf key and merge the single branch
                normalized.remove(*key);
                for (k, v) in non_null_branches.remove(0) {
                    normalized.entry(k).or_insert(v);
                }
                normalized.insert("nullable".to_string(), Value::Bool(true));
                break;
            }
        }
    }

    // Recursively normalize properties
    if let Some(Value::Object(props)) = normalized.get_mut("properties") {
        let new_props: Map<String, Value> = props
            .iter()
            .map(|(name, prop)| (name.clone(), normalize_schema_for_openai(prop.clone())))
            .collect();
        *props = new_props;
    }

    // Recursively normalize items
    if let Some(items) = normalized.get("items").cloned() {
        let normalized_items = normalize_schema_for_openai(items);
        normalized.insert("items".to_string(), normalized_items);
    }

    // Ensure object type has properties and required
    if normalized.get("type").and_then(|v| v.as_str()) == Some("object") {
        normalized
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()));
        normalized
            .entry("required")
            .or_insert_with(|| Value::Array(Vec::new()));
    }

    Value::Object(normalized)
}

/// Wraps a single MCP server tool as a native ombudsman Tool.
pub struct McpToolWrapper {
    /// Namespaced name: `mcp_<server>_<tool_name>`
    name: String,
    /// Raw MCP tool name (used when calling the server)
    original_name: String,
    description: String,
    parameters: Value,
    /// rmcp Peer (client handle) used to call tools
    peer: Arc<rmcp::Peer<rmcp::RoleClient>>,
    tool_timeout: u64,
}

impl McpToolWrapper {
    pub fn new(
        peer: Arc<rmcp::Peer<rmcp::RoleClient>>,
        server_name: &str,
        tool: &rmcp::model::Tool,
        tool_timeout: u32,
    ) -> Self {
        let original_name = tool.name.as_ref().to_string();
        let name = format!("mcp_{}_{}", server_name, original_name);
        let description = tool
            .description
            .as_deref()
            .unwrap_or(&original_name)
            .to_string();
        let raw_schema = Value::Object(tool.input_schema.as_ref().clone());
        let parameters = normalize_schema_for_openai(raw_schema);
        Self {
            name,
            original_name,
            description,
            parameters,
            peer,
            tool_timeout: tool_timeout as u64,
        }
    }

    /// The original (non-namespaced) MCP tool name.
    pub fn original_name(&self) -> &str {
        &self.original_name
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(McpToolWrapper {
            name: self.name.clone(),
            original_name: self.original_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            peer: Arc::clone(&self.peer),
            tool_timeout: self.tool_timeout,
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        // Convert args map to JsonObject
        let arguments: Map<String, Value> = args.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let params = CallToolRequestParams::new(self.original_name.clone())
            .with_arguments(arguments);

        let result = tokio::time::timeout(
            tokio::time::Duration::from_secs(self.tool_timeout),
            self.peer.call_tool(params),
        )
        .await;

        match result {
            Err(_elapsed) => {
                warn!(
                    "MCP tool '{}' timed out after {}s",
                    self.name, self.tool_timeout
                );
                format!("(MCP tool call timed out after {}s)", self.tool_timeout)
            }
            Ok(Err(e)) => {
                warn!("MCP tool '{}' failed: {}", self.name, e);
                format!("(MCP tool call failed: {})", e)
            }
            Ok(Ok(call_result)) => {
                let parts: Vec<String> = call_result
                    .content
                    .iter()
                    .map(|content| {
                        if let Some(text) = content.as_text() {
                            text.text.clone()
                        } else {
                            format!("{:?}", content)
                        }
                    })
                    .collect();
                let output = parts.join("\n");
                if output.is_empty() {
                    "(no output)".to_string()
                } else {
                    output
                }
            }
        }
    }
}

/// Build custom headers map from string k/v pairs.
fn build_headers(
    raw: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>, anyhow::Error> {
    let mut headers = HashMap::new();
    for (name, value) in raw {
        let header_name = HeaderName::from_str(name)
            .map_err(|e| anyhow::anyhow!("invalid header name '{}': {}", name, e))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|e| anyhow::anyhow!("invalid header value for '{}': {}", name, e))?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

/// Detect transport type from URL when type is not explicitly set.
///
/// Convention: URLs ending with `/sse` are treated as legacy SSE; everything else is streamableHttp.
fn detect_transport_type(url: &str) -> McpTransportType {
    if url.trim_end_matches('/').ends_with("/sse") {
        McpTransportType::Sse
    } else {
        McpTransportType::StreamableHttp
    }
}

/// A connected MCP session that holds the running client service.
///
/// Dropping this closes the connection.
pub struct McpSession {
    /// The running rmcp client service (background task).
    _service: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    /// The peer handle used to make requests.
    pub peer: Arc<rmcp::Peer<rmcp::RoleClient>>,
}

/// Connect to all configured MCP HTTP servers and register their tools.
///
/// Returns a list of active sessions that must be kept alive for as long as
/// the tools are in use. Dropping a session closes its connection.
///
/// Only `sse` and `streamableHttp` transport types are supported.
/// Entries with neither a URL nor a valid transport type are skipped.
pub async fn connect_mcp_servers(
    mcp_servers: &HashMap<String, McpServerConfig>,
    registry: &mut ToolRegistry,
) -> Vec<McpSession> {
    let mut sessions: Vec<McpSession> = Vec::new();

    for (name, cfg) in mcp_servers {
        if cfg.url.trim().is_empty() {
            warn!(
                "MCP server '{}': no URL configured (stdio is not supported), skipping",
                name
            );
            continue;
        }

        let transport_type = cfg
            .transport_type
            .clone()
            .unwrap_or_else(|| detect_transport_type(&cfg.url));

        // Both sse and streamableHttp are handled by StreamableHttpClientTransport.
        // Legacy SSE servers can be connected by specifying the /sse endpoint URL.
        if matches!(transport_type, McpTransportType::Sse) {
            debug!("MCP server '{}': using SSE transport via streamable HTTP client", name);
        }

        // Build custom headers
        let custom_headers = match build_headers(&cfg.headers) {
            Ok(h) => h,
            Err(e) => {
                warn!("MCP server '{}': invalid headers: {}, skipping", name, e);
                continue;
            }
        };

        let config = StreamableHttpClientTransportConfig {
            uri: cfg.url.clone().into(),
            custom_headers,
            ..Default::default()
        };

        let transport = StreamableHttpClientTransport::from_config(config);

        let service = match ().serve(transport).await {
            Ok(s) => s,
            Err(e) => {
                warn!("MCP server '{}': failed to connect: {}", name, e);
                continue;
            }
        };

        let peer = Arc::new(service.peer().clone());

        // List tools
        let tools_result = match service.peer().list_all_tools().await {
            Ok(t) => t,
            Err(e) => {
                warn!("MCP server '{}': failed to list tools: {}", name, e);
                sessions.push(McpSession {
                    _service: service,
                    peer,
                });
                continue;
            }
        };

        let enabled_tools: std::collections::HashSet<String> =
            cfg.enabled_tools.iter().cloned().collect();
        let allow_all = enabled_tools.contains("*");

        let mut registered_count = 0usize;
        let mut matched_enabled_tools: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let available_raw: Vec<String> = tools_result.iter().map(|t| t.name.to_string()).collect();
        let available_wrapped: Vec<String> = tools_result
            .iter()
            .map(|t| format!("mcp_{}_{}", name, t.name))
            .collect();

        for tool in &tools_result {
            let raw_name = tool.name.as_ref();
            let wrapped_name = format!("mcp_{}_{}", name, raw_name);

            if !allow_all
                && !enabled_tools.contains(raw_name)
                && !enabled_tools.contains(&wrapped_name)
            {
                debug!(
                    "MCP: skipping tool '{}' from server '{}' (not in enabledTools)",
                    wrapped_name, name
                );
                continue;
            }

            let wrapper = McpToolWrapper::new(
                Arc::clone(&peer),
                name,
                tool,
                cfg.tool_timeout,
            );
            debug!("MCP: registered tool '{}' from server '{}'", wrapper.name(), name);
            registry.register(Box::new(wrapper));
            registered_count += 1;

            if !allow_all {
                if enabled_tools.contains(raw_name) {
                    matched_enabled_tools.insert(raw_name.to_string());
                }
                if enabled_tools.contains(&wrapped_name) {
                    matched_enabled_tools.insert(wrapped_name);
                }
            }
        }

        if !allow_all {
            let unmatched: Vec<String> = enabled_tools
                .difference(&matched_enabled_tools)
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if !unmatched.is_empty() {
                warn!(
                    "MCP server '{}': enabledTools entries not found: {}. \
                    Available raw names: {}. Available wrapped names: {}",
                    name,
                    unmatched.join(", "),
                    if available_raw.is_empty() {
                        "(none)".to_string()
                    } else {
                        available_raw.join(", ")
                    },
                    if available_wrapped.is_empty() {
                        "(none)".to_string()
                    } else {
                        available_wrapped.join(", ")
                    },
                );
            }
        }

        info!(
            "MCP server '{}': connected, {} tool(s) registered",
            name, registered_count
        );

        sessions.push(McpSession {
            _service: service,
            peer,
        });
    }

    sessions
}
