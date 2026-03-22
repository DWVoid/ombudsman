//! Integration tests for ombudsman.

use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

// ---- Session tests ----

mod session_tests {
    use super::*;
    use ombudsman::session::{Session, SessionManager};
    use serde_json::json;

    #[test]
    fn test_session_new() {
        let session = Session::new("test:key");
        assert_eq!(session.key, "test:key");
        assert!(session.messages.is_empty());
        assert_eq!(session.last_consolidated, 0);
    }

    #[test]
    fn test_session_get_history_empty() {
        let session = Session::new("test:key");
        let history = session.get_history(0);
        assert!(history.is_empty());
    }

    #[test]
    fn test_session_get_history_with_messages() {
        let mut session = Session::new("test:key");
        let mut msg = HashMap::new();
        msg.insert("role".to_string(), json!("user"));
        msg.insert("content".to_string(), json!("Hello"));
        session.messages.push(msg);

        let history = session.get_history(0);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["role"], json!("user"));
    }

    #[test]
    fn test_session_clear() {
        let mut session = Session::new("test:key");
        let mut msg = HashMap::new();
        msg.insert("role".to_string(), json!("user"));
        msg.insert("content".to_string(), json!("Hello"));
        session.messages.push(msg);
        session.last_consolidated = 1;

        session.clear();
        assert!(session.messages.is_empty());
        assert_eq!(session.last_consolidated, 0);
    }

    #[test]
    fn test_session_manager_save_load() {
        let dir = TempDir::new().unwrap();
        let mut manager = SessionManager::new(dir.path()).unwrap();

        let session = manager.get_or_create("test:chat");
        let mut msg = HashMap::new();
        msg.insert("role".to_string(), json!("user"));
        msg.insert("content".to_string(), json!("Hello, world!"));
        session.messages.push(msg);
        let session_clone = session.clone();
        manager.save(&session_clone);

        // Create a new manager to test persistence
        let mut manager2 = SessionManager::new(dir.path()).unwrap();
        let loaded = manager2.get_or_create("test:chat");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0]["content"], json!("Hello, world!"));
    }

    #[test]
    fn test_session_legal_start_clean() {
        let mut session = Session::new("test");
        // A clean conversation should have start=0
        let mut u = HashMap::new();
        u.insert("role".to_string(), json!("user"));
        u.insert("content".to_string(), json!("Q"));
        session.messages.push(u);

        let history = session.get_history(0);
        assert_eq!(history.len(), 1);
    }
}

// ---- Config tests ----

mod config_tests {
    use ombudsman::config::schema::{AgentDefaults, Config};

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.agents.defaults.max_tool_iterations, 40);
        assert_eq!(config.agents.defaults.context_window_tokens, 65_536);
        assert!(config.agents.defaults.temperature < 1.0);
    }

    #[test]
    fn test_config_deserialization() {
        let json = r#"{
            "agents": {
                "defaults": {
                    "model": "gpt-4o",
                    "maxTokens": 8192
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-4o");
        assert_eq!(config.agents.defaults.max_tokens, 8192);
    }

    #[test]
    fn test_config_save_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.json");

        let config = Config::default();
        let content = serde_json::to_string_pretty(&config).unwrap();
        std::fs::write(&config_path, content).unwrap();

        let loaded: Config = serde_json::from_str(
            &std::fs::read_to_string(&config_path).unwrap()
        ).unwrap();
        assert_eq!(loaded.agents.defaults.max_tool_iterations, config.agents.defaults.max_tool_iterations);
    }
}

// ---- Bus tests ----

mod bus_tests {
    use ombudsman::bus::{InboundMessage, MessageBus, OutboundMessage};

    #[tokio::test]
    async fn test_message_bus_roundtrip() {
        let bus = MessageBus::new(16);

        let msg = InboundMessage::new("cli", "user1", "chat1", "Hello");
        bus.publish_inbound(msg).await.unwrap();

        let received = bus.consume_inbound().await.unwrap();
        assert_eq!(received.content, "Hello");
        assert_eq!(received.channel, "cli");
    }

    #[tokio::test]
    async fn test_outbound_message() {
        let bus = MessageBus::new(16);
        let out = OutboundMessage::new("cli", "chat1", "Response");
        bus.publish_outbound(out).await.unwrap();

        let received = bus.consume_outbound().await.unwrap();
        assert_eq!(received.content, "Response");
    }

    #[test]
    fn test_inbound_session_key() {
        let msg = InboundMessage::new("telegram", "user123", "group456", "test");
        assert_eq!(msg.session_key(), "telegram:group456");
    }

    #[test]
    fn test_inbound_session_key_override() {
        let msg = InboundMessage {
            channel: "telegram".to_string(),
            sender_id: "user123".to_string(),
            chat_id: "group456".to_string(),
            content: "test".to_string(),
            timestamp: chrono::Local::now(),
            media: Vec::new(),
            metadata: std::collections::HashMap::new(),
            session_key_override: Some("custom:session".to_string()),
        };
        assert_eq!(msg.session_key(), "custom:session");
    }
}

// ---- Tool tests ----

mod tool_tests {
    use ombudsman::agent::tools::filesystem::{ListDirTool, ReadFileTool, WriteFileTool};
    use ombudsman::agent::tools::base::Tool;
    use std::collections::HashMap;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_and_read_file() {
        let dir = TempDir::new().unwrap();
        let writer = WriteFileTool::new(Some(dir.path().to_path_buf()), None);
        let reader = ReadFileTool::new(Some(dir.path().to_path_buf()), None);

        let mut args = HashMap::new();
        args.insert("path".to_string(), json!("test.txt"));
        args.insert("content".to_string(), json!("Hello, Rust!"));

        let result = writer.execute(&args).await;
        assert!(result.contains("Successfully wrote"), "Expected success, got: {}", result);

        let mut read_args = HashMap::new();
        read_args.insert("path".to_string(), json!("test.txt"));

        let content = reader.execute(&read_args).await;
        assert!(content.contains("Hello, Rust!"), "Expected content, got: {}", content);
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let reader = ReadFileTool::new(Some(dir.path().to_path_buf()), None);

        let mut args = HashMap::new();
        args.insert("path".to_string(), json!("nonexistent.txt"));

        let result = reader.execute(&args).await;
        assert!(result.contains("Error"), "Expected error, got: {}", result);
    }

    #[tokio::test]
    async fn test_list_dir() {
        let dir = TempDir::new().unwrap();
        // Create some files
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();

        let lister = ListDirTool::new(Some(dir.path().to_path_buf()), None);
        let mut args = HashMap::new();
        args.insert("path".to_string(), json!(dir.path().to_str().unwrap()));

        let result = lister.execute(&args).await;
        assert!(result.contains("a.txt"), "Expected a.txt in listing: {}", result);
        assert!(result.contains("b.txt"), "Expected b.txt in listing: {}", result);
    }

    #[tokio::test]
    async fn test_list_nonexistent_dir() {
        let lister = ListDirTool::new(None, None);
        let mut args = HashMap::new();
        args.insert("path".to_string(), json!("/nonexistent/path/xyz"));

        let result = lister.execute(&args).await;
        assert!(result.contains("Error"), "Expected error, got: {}", result);
    }
}

// ---- Provider tests ----

mod provider_tests {
    use ombudsman::providers::base::is_transient_error;

    #[test]
    fn test_transient_error_detection() {
        assert!(is_transient_error(Some("429 rate limit exceeded")));
        assert!(is_transient_error(Some("503 service unavailable")));
        assert!(is_transient_error(Some("connection timeout")));
        assert!(!is_transient_error(Some("400 bad request")));
        assert!(!is_transient_error(Some("invalid api key")));
        assert!(!is_transient_error(None));
    }
}

// ---- Memory tests ----

mod memory_tests {
    use ombudsman::agent::memory::MemoryStore;
    use tempfile::TempDir;

    #[test]
    fn test_memory_store_read_write() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path());

        assert_eq!(store.read_long_term(), "");

        store.write_long_term("# Memory\n\nSome facts here.");
        assert_eq!(store.read_long_term(), "# Memory\n\nSome facts here.");
    }

    #[test]
    fn test_memory_store_append_history() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path());

        store.append_history("[2025-01-01 10:00] USER: Hello");
        store.append_history("[2025-01-01 10:01] ASSISTANT: Hi!");

        let history_path = dir.path().join("memory").join("HISTORY.md");
        let content = std::fs::read_to_string(&history_path).unwrap();
        assert!(content.contains("[2025-01-01 10:00]"));
        assert!(content.contains("[2025-01-01 10:01]"));
    }

    #[test]
    fn test_memory_store_context_empty() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path());
        assert_eq!(store.get_memory_context(), "");
    }

    #[test]
    fn test_memory_store_context_with_content() {
        let dir = TempDir::new().unwrap();
        let store = MemoryStore::new(dir.path());
        store.write_long_term("Important facts.");
        let ctx = store.get_memory_context();
        assert!(ctx.contains("Long-term Memory"));
        assert!(ctx.contains("Important facts."));
    }
}

// ---- Context tests ----

mod context_tests {
    use ombudsman::agent::context::ContextBuilder;
    use tempfile::TempDir;

    #[test]
    fn test_context_builder_system_prompt() {
        let dir = TempDir::new().unwrap();
        let builder = ContextBuilder::new(dir.path());
        let prompt = builder.build_system_prompt();
        assert!(prompt.contains("ombudsman"), "Expected ombudsman in prompt");
        assert!(prompt.contains("Workspace"), "Expected Workspace in prompt");
    }

    #[test]
    fn test_context_builder_messages() {
        let dir = TempDir::new().unwrap();
        let builder = ContextBuilder::new(dir.path());

        let history = vec![];
        let messages = builder.build_messages(
            &history,
            "What is the weather?",
            None,
            Some("cli"),
            Some("default"),
            "user",
        );

        assert!(!messages.is_empty());
        assert_eq!(messages[0]["role"], "system");

        let last = messages.last().unwrap();
        assert_eq!(last["role"], "user");
        let content = last["content"].as_str().unwrap_or("");
        assert!(content.contains("What is the weather?"), "User content: {}", content);
    }

    #[test]
    fn test_add_tool_result() {
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "System prompt"}),
            serde_json::json!({"role": "user", "content": "Question"}),
        ];

        ContextBuilder::add_tool_result(&mut messages, "call_123", "exec", "stdout output");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_123");
        assert_eq!(messages[2]["content"], "stdout output");
    }

    #[test]
    fn test_add_assistant_message() {
        let mut messages = vec![];
        ContextBuilder::add_assistant_message(
            &mut messages,
            Some("I will help you."),
            None,
            None,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"], "I will help you.");
    }
}

// ---- MCP tool tests ----

mod mcp_tests {
    use ombudsman::agent::normalize_schema_for_openai;
    use serde_json::{json, Value};

    // --- Schema normalization tests (functionally equivalent to nanobot Python test suite) ---

    #[test]
    fn test_normalize_non_nullable_anyof_untouched() {
        let schema = json!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [{"type": "string"}, {"type": "integer"}]
                }
            },
            "required": []
        });
        let result = normalize_schema_for_openai(schema);
        // non-nullable anyOf is left alone
        assert_eq!(
            result["properties"]["value"]["anyOf"],
            json!([{"type": "string"}, {"type": "integer"}])
        );
    }

    #[test]
    fn test_normalize_type_array_nullable() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": ["string", "null"]}
            },
            "required": []
        });
        let result = normalize_schema_for_openai(schema);
        assert_eq!(result["properties"]["name"]["type"], json!("string"));
        assert_eq!(result["properties"]["name"]["nullable"], json!(true));
    }

    #[test]
    fn test_normalize_anyof_nullable_property() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "anyOf": [{"type": "string"}, {"type": "null"}],
                    "description": "optional name"
                }
            },
            "required": []
        });
        let result = normalize_schema_for_openai(schema);
        let prop = &result["properties"]["name"];
        assert_eq!(prop["type"], json!("string"));
        assert_eq!(prop["description"], json!("optional name"));
        assert_eq!(prop["nullable"], json!(true));
        // anyOf should be removed after normalization
        assert!(prop.get("anyOf").is_none());
    }

    #[test]
    fn test_normalize_oneof_nullable_property() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": {
                    "oneOf": [{"type": "integer"}, {"type": "null"}]
                }
            },
            "required": []
        });
        let result = normalize_schema_for_openai(schema);
        let prop = &result["properties"]["count"];
        assert_eq!(prop["type"], json!("integer"));
        assert_eq!(prop["nullable"], json!(true));
        assert!(prop.get("oneOf").is_none());
    }

    #[test]
    fn test_normalize_non_object_schema_passthrough() {
        let schema = json!({"type": "string"});
        let result = normalize_schema_for_openai(schema.clone());
        assert_eq!(result, schema);
    }

    #[test]
    fn test_normalize_object_gets_defaults() {
        let schema = json!({"type": "object"});
        let result = normalize_schema_for_openai(schema);
        // An empty object schema gets default properties and required
        assert!(result["properties"].is_object());
        assert!(result["required"].is_array());
    }

    #[test]
    fn test_normalize_nested_items() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": {"type": ["string", "null"]}
                }
            },
            "required": []
        });
        let result = normalize_schema_for_openai(schema);
        let items = &result["properties"]["tags"]["items"];
        assert_eq!(items["type"], json!("string"));
        assert_eq!(items["nullable"], json!(true));
    }

    #[test]
    fn test_normalize_non_exhaustive_anyof_left_alone() {
        // anyOf with two non-null types should not be normalized
        let schema = json!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [{"type": "string"}, {"type": "boolean"}]
                }
            },
            "required": []
        });
        let result = normalize_schema_for_openai(schema);
        let prop = &result["properties"]["value"];
        // anyOf should be preserved since there's no null branch
        assert_eq!(
            prop["anyOf"],
            json!([{"type": "string"}, {"type": "boolean"}])
        );
        assert!(prop.get("nullable").is_none());
    }

    #[test]
    fn test_config_mcp_server_defaults() {
        use ombudsman::config::schema::McpServerConfig;
        let cfg = McpServerConfig::default();
        assert_eq!(cfg.enabled_tools, vec!["*"]);
        assert_eq!(cfg.tool_timeout, 30);
        assert!(cfg.url.is_empty());
        assert!(cfg.headers.is_empty());
    }

    #[test]
    fn test_config_mcp_server_deserialization() {
        use ombudsman::config::schema::{McpServerConfig, McpTransportType};
        let json = r#"{
            "url": "https://example.com/mcp",
            "type": "streamableHttp",
            "toolTimeout": 60,
            "enabledTools": ["search", "lookup"],
            "headers": {"Authorization": "Bearer token123"}
        }"#;
        let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.url, "https://example.com/mcp");
        assert_eq!(cfg.transport_type, Some(McpTransportType::StreamableHttp));
        assert_eq!(cfg.tool_timeout, 60);
        assert_eq!(cfg.enabled_tools, vec!["search", "lookup"]);
        assert_eq!(cfg.headers.get("Authorization").unwrap(), "Bearer token123");
    }

    #[test]
    fn test_config_mcp_server_sse_type() {
        use ombudsman::config::schema::{McpServerConfig, McpTransportType};
        let json = r#"{"url": "http://localhost:8080/sse", "type": "sse"}"#;
        let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.transport_type, Some(McpTransportType::Sse));
    }

    #[test]
    fn test_tools_config_mcp_servers_empty_by_default() {
        use ombudsman::config::schema::ToolsConfig;
        let cfg = ToolsConfig::default();
        assert!(cfg.mcp_servers.is_empty());
    }

    #[test]
    fn test_tools_config_with_mcp_servers() {
        use ombudsman::config::schema::ToolsConfig;
        let json = r#"{
            "mcpServers": {
                "myserver": {
                    "url": "http://localhost:3000/mcp",
                    "enabledTools": ["*"]
                }
            }
        }"#;
        let cfg: ToolsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mcp_servers.len(), 1);
        let server = cfg.mcp_servers.get("myserver").unwrap();
        assert_eq!(server.url, "http://localhost:3000/mcp");
        assert_eq!(server.enabled_tools, vec!["*"]);
    }
}
