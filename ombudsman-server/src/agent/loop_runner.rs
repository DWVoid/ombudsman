//! Core agent loop: the central processing engine.

use crate::agent::context::ContextBuilder;
use crate::agent::memory::MemoryConsolidator;
use crate::agent::subagent::SubagentManager;
use crate::agent::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use crate::agent::tools::mcp::{McpSession, connect_mcp_servers};
use crate::agent::tools::message::{MessageTool, MessageToolContext};
use crate::agent::tools::registry::ToolRegistry;
use crate::agent::tools::shell::ExecTool;
use crate::agent::tools::spawn::{SpawnTool, SpawnToolContext};
use crate::agent::tools::web::{WebFetchTool, WebSearchTool};
use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::config::schema::{ChannelsConfig, ExecToolConfig, McpServerConfig, WebSearchConfig};
use crate::providers::base::LLMProvider;
use crate::session::{Session, SessionManager};
use chrono::Local;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{info, warn};

const TOOL_RESULT_MAX_CHARS: usize = 16_000;

pub struct AgentLoop {
    bus: Arc<MessageBus>,
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    model: String,
    max_iterations: u32,
    context_window_tokens: u32,
    context: ContextBuilder,
    sessions: Mutex<SessionManager>,
    /// Base tool registry (built-in tools). MCP tools are added lazily on first use.
    base_tools: ToolRegistry,
    /// Combined tool registry after MCP connection (None until first message).
    tools: Mutex<Option<Arc<ToolRegistry>>>,
    memory: Mutex<MemoryConsolidator>,
    channels_config: ChannelsConfig,
    start_time: Instant,
    last_usage: Mutex<HashMap<String, u64>>,
    running: Mutex<bool>,
    /// MCP server configurations.
    mcp_servers: HashMap<String, McpServerConfig>,
    /// Active MCP sessions (kept alive to maintain connections).
    mcp_sessions: Mutex<Vec<McpSession>>,
    /// Whether MCP connection has been attempted.
    mcp_connected: Mutex<bool>,
    /// Subagent manager for background task execution.
    subagent_manager: Arc<SubagentManager>,
    /// Shared context for SpawnTool (channel/chat_id updated per message).
    spawn_ctx: Arc<StdMutex<SpawnToolContext>>,
    /// Shared context for MessageTool (channel/chat_id updated per message).
    message_ctx: Arc<StdMutex<MessageToolContext>>,
}

impl AgentLoop {
    pub fn new(
        bus: Arc<MessageBus>,
        provider: Arc<dyn LLMProvider>,
        workspace: &Path,
        model: Option<String>,
        max_iterations: u32,
        context_window_tokens: u32,
        web_search_config: WebSearchConfig,
        web_proxy: Option<String>,
        exec_config: ExecToolConfig,
        restrict_to_workspace: bool,
        channels_config: ChannelsConfig,
        mcp_servers: HashMap<String, McpServerConfig>,
    ) -> anyhow::Result<Self> {
        let model = model.unwrap_or_else(|| provider.get_default_model().to_string());
        let context = ContextBuilder::new(workspace);
        let sessions = Mutex::new(SessionManager::new(workspace)?);
        let memory = Mutex::new(MemoryConsolidator::new(workspace, context_window_tokens));

        let mut base_tools = ToolRegistry::new();

        let allowed_dir = if restrict_to_workspace {
            Some(workspace.to_path_buf())
        } else {
            None
        };

        base_tools.register(Box::new(ReadFileTool::new(
            Some(workspace.to_path_buf()),
            allowed_dir.clone(),
        )));
        base_tools.register(Box::new(WriteFileTool::new(
            Some(workspace.to_path_buf()),
            allowed_dir.clone(),
        )));
        base_tools.register(Box::new(EditFileTool::new(
            Some(workspace.to_path_buf()),
            allowed_dir.clone(),
        )));
        base_tools.register(Box::new(ListDirTool::new(
            Some(workspace.to_path_buf()),
            allowed_dir.clone(),
        )));

        if exec_config.enable {
            base_tools.register(Box::new(ExecTool::new(
                exec_config.timeout as u64,
                Some(workspace.display().to_string()),
                restrict_to_workspace,
                exec_config.path_append.clone(),
            )));
        }

        base_tools.register(Box::new(WebSearchTool::new(
            &web_search_config.provider,
            web_search_config.api_key.clone(),
            web_search_config.base_url.clone(),
            web_search_config.max_results,
            web_proxy.clone(),
        )));

        base_tools.register(Box::new(WebFetchTool::new(50_000, web_proxy.clone())));

        // Build SubagentManager and register SpawnTool + MessageTool
        let subagent_manager = Arc::new(SubagentManager::new(
            Arc::clone(&provider),
            workspace,
            Arc::clone(&bus),
            model.clone(),
            web_search_config.clone(),
            web_proxy.clone(),
            exec_config.clone(),
            restrict_to_workspace,
        ));

        let spawn_ctx = Arc::new(StdMutex::new(SpawnToolContext::default()));
        let message_ctx = Arc::new(StdMutex::new(MessageToolContext::default()));

        let spawn_tool = SpawnTool::new(Arc::clone(&subagent_manager), Arc::clone(&spawn_ctx));
        base_tools.register(Box::new(spawn_tool));

        let message_tool = MessageTool::new(bus.outbound_sender(), Arc::clone(&message_ctx));
        base_tools.register(Box::new(message_tool));

        Ok(Self {
            bus,
            provider,
            workspace: workspace.to_path_buf(),
            model,
            max_iterations,
            context_window_tokens,
            context,
            sessions,
            base_tools,
            tools: Mutex::new(None),
            memory,
            channels_config,
            start_time: Instant::now(),
            last_usage: Mutex::new(HashMap::new()),
            running: Mutex::new(false),
            mcp_servers,
            mcp_sessions: Mutex::new(Vec::new()),
            mcp_connected: Mutex::new(false),
            subagent_manager,
            spawn_ctx,
            message_ctx,
        })
    }

    /// Lazily connect to configured MCP servers on first use.
    ///
    /// If there are no MCP servers configured this is a no-op.
    /// On failure the error is logged and processing continues without MCP tools
    /// (a retry will be attempted on the next message).
    async fn connect_mcp(&self) {
        if self.mcp_servers.is_empty() {
            return;
        }

        let mut connected = self.mcp_connected.lock().await;
        if *connected {
            return;
        }

        info!("Connecting to {} MCP server(s)...", self.mcp_servers.len());

        // Build a fresh combined registry (clone built-in tools, then add MCP tools)
        let mut combined = self.base_tools.clone_registry();
        let sessions = connect_mcp_servers(&self.mcp_servers, &mut combined).await;

        let session_count = sessions.len();
        *self.mcp_sessions.lock().await = sessions;
        *self.tools.lock().await = Some(Arc::new(combined));
        *connected = true;

        info!(
            "MCP connection complete: {} session(s) established",
            session_count
        );
    }

    /// Get (or build) the active tool registry.
    async fn get_tools(&self) -> Arc<ToolRegistry> {
        let guard = self.tools.lock().await;
        if let Some(ref registry) = *guard {
            return Arc::clone(registry);
        }
        drop(guard);
        // No MCP configured — wrap base tools in an Arc directly
        Arc::new(self.base_tools.clone_registry())
    }

    /// Close all active MCP sessions and reset state.
    pub async fn close_mcp(&self) {
        let mut sessions = self.mcp_sessions.lock().await;
        sessions.clear();
        *self.mcp_connected.lock().await = false;
        *self.tools.lock().await = None;
        info!("MCP sessions closed");
    }

    /// Strip <think>...</think> blocks from LLM output.
    fn strip_think(text: Option<&str>) -> Option<String> {
        let text = text?;
        let re = Regex::new(r"(?s)<think>.*?</think>").unwrap();
        let result = re.replace_all(text, "").trim().to_string();
        if result.is_empty() { None } else { Some(result) }
    }

    /// Format tool calls as a concise hint string.
    fn tool_hint(tool_calls: &[crate::providers::base::ToolCallRequest]) -> String {
        tool_calls
            .iter()
            .map(|tc| {
                let val = tc.arguments.values().next()
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.len() > 40 {
                            format!("\"{}…\"", &s[..40])
                        } else {
                            format!("\"{}\"", s)
                        }
                    });
                match val {
                    Some(v) => format!("{}({})", tc.name, v),
                    None => tc.name.clone(),
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Run the agent iteration loop.
    async fn run_agent_loop(
        &self,
        initial_messages: Vec<Value>,
        on_progress: Option<Arc<dyn Fn(String, bool) + Send + Sync>>,
    ) -> (Option<String>, Vec<String>, Vec<Value>) {
        let mut messages = initial_messages;
        let mut iteration = 0u32;
        let mut final_content: Option<String> = None;
        let mut tools_used: Vec<String> = Vec::new();

        // Capture the current tool registry snapshot for this loop invocation
        let tools = self.get_tools().await;

        while iteration < self.max_iterations {
            iteration += 1;

            let tool_defs = tools.get_definitions();
            let response = self
                .provider
                .chat_with_retry(
                    &messages,
                    Some(&tool_defs),
                    Some(&self.model),
                    None,
                    None,
                    None,
                    None,
                )
                .await;

            // Update usage stats
            {
                let mut usage = self.last_usage.lock().await;
                *usage = response.usage.clone();
            }

            if response.has_tool_calls() {
                // Stream progress if callback provided
                if let Some(cb) = &on_progress {
                    if let Some(thought) = Self::strip_think(response.content.as_deref()) {
                        cb(thought, false);
                    }
                    let hint = Self::tool_hint(&response.tool_calls);
                    if let Some(cleaned) = Self::strip_think(Some(&hint)) {
                        cb(cleaned, true);
                    }
                }

                let tool_call_dicts: Vec<Value> = response
                    .tool_calls
                    .iter()
                    .map(|tc| tc.to_openai_tool_call())
                    .collect();

                ContextBuilder::add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(&tool_call_dicts),
                    response.reasoning_content.as_deref(),
                );

                for tool_call in &response.tool_calls {
                    tools_used.push(tool_call.name.clone());
                    let args_str = serde_json::to_string(&tool_call.arguments).unwrap_or_default();
                    info!("Tool call: {}({})", tool_call.name, &args_str[..args_str.len().min(200)]);

                    let result = tools.execute(&tool_call.name, &tool_call.arguments).await;
                    ContextBuilder::add_tool_result(
                        &mut messages,
                        &tool_call.id,
                        &tool_call.name,
                        &result,
                    );
                }
            } else {
                let clean = Self::strip_think(response.content.as_deref());

                if response.finish_reason == "error" {
                    warn!("LLM returned error: {:?}", &clean);
                    final_content = Some(
                        clean.unwrap_or_else(|| "Sorry, I encountered an error calling the AI model.".to_string())
                    );
                    break;
                }

                ContextBuilder::add_assistant_message(
                    &mut messages,
                    clean.as_deref(),
                    None,
                    response.reasoning_content.as_deref(),
                );
                final_content = clean;
                break;
            }
        }

        if final_content.is_none() && iteration >= self.max_iterations {
            warn!("Max iterations ({}) reached", self.max_iterations);
            final_content = Some(format!(
                "I reached the maximum number of tool call iterations ({}) without completing the task. You can try breaking the task into smaller steps.",
                self.max_iterations
            ));
        }

        (final_content, tools_used, messages)
    }

    /// Save new turn messages to the session.
    fn save_turn(&self, session: &mut Session, messages: &[Value], skip: usize) {
        let now = Local::now();
        for m in messages.iter().skip(skip) {
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = m.get("content");

            // Skip empty assistant messages
            if role == "assistant" && content.is_none() && m.get("tool_calls").is_none() {
                continue;
            }

            let mut entry: HashMap<String, Value> = HashMap::new();
            for (k, v) in m.as_object().into_iter().flatten() {
                entry.insert(k.clone(), v.clone());
            }

            // Truncate large tool results
            if role == "tool" {
                if let Some(Value::String(s)) = entry.get("content") {
                    if s.len() > TOOL_RESULT_MAX_CHARS {
                        let truncated = format!("{}\n... (truncated)", &s[..TOOL_RESULT_MAX_CHARS]);
                        entry.insert("content".to_string(), json!(truncated));
                    }
                }
            }

            // Strip runtime context prefix from user messages
            if role == "user" {
                if let Some(Value::String(s)) = entry.get("content") {
                    let tag = ContextBuilder::runtime_context_tag();
                    if s.starts_with(tag) {
                        let parts: Vec<&str> = s.splitn(2, "\n\n").collect();
                        if parts.len() > 1 && !parts[1].trim().is_empty() {
                            entry.insert("content".to_string(), json!(parts[1]));
                        } else {
                            continue;
                        }
                    }
                }
            }

            entry.entry("timestamp".to_string())
                .or_insert_with(|| json!(now.to_rfc3339()));
            session.messages.push(entry);
        }
        session.updated_at = Local::now();
    }

    async fn build_status_content(&self, session: &Session, ctx_est: u64) -> String {
        let uptime_s = self.start_time.elapsed().as_secs();
        let uptime = if uptime_s >= 3600 {
            format!("{}h {}m", uptime_s / 3600, (uptime_s % 3600) / 60)
        } else {
            format!("{}m {}s", uptime_s / 60, uptime_s % 60)
        };
        let usage = self.last_usage.lock().await;
        let last_in = usage.get("prompt_tokens").copied().unwrap_or(0);
        let last_out = usage.get("completion_tokens").copied().unwrap_or(0);
        let ctx_total = self.context_window_tokens as u64;
        let ctx_pct = if ctx_total > 0 { (ctx_est * 100) / ctx_total } else { 0 };
        let ctx_used = if ctx_est >= 1000 { format!("{}k", ctx_est / 1000) } else { ctx_est.to_string() };
        let ctx_total_str = if ctx_total > 0 { format!("{}k", ctx_total / 1024) } else { "n/a".to_string() };
        let msg_count = session.get_history(0).len();
        format!(
            "🐈 ombudsman v{}\n🧠 Model: {}\n📊 Tokens: {} in / {} out\n📚 Context: {}/{} ({}%)\n💬 Session: {} messages\n⏱ Uptime: {}",
            env!("CARGO_PKG_VERSION"),
            self.model,
            last_in, last_out,
            ctx_used, ctx_total_str, ctx_pct,
            msg_count,
            uptime
        )
    }

    /// Process a single inbound message and return the response.
    pub async fn process_message(
        &self,
        msg: &InboundMessage,
        on_progress: Option<Arc<dyn Fn(String, bool) + Send + Sync>>,
    ) -> Option<OutboundMessage> {
        let preview = if msg.content.len() > 80 {
            format!("{}...", &msg.content[..80])
        } else {
            msg.content.clone()
        };
        info!("Processing message from {}:{}: {}", msg.channel, msg.sender_id, preview);

        let session_key = msg.session_key();

        // Handle slash commands
        let cmd = msg.content.trim().to_lowercase();
        match cmd.as_str() {
            "/new" => {
                let mut sessions = self.sessions.lock().await;
                let session = sessions.get_or_create(&session_key);
                let snapshot: Vec<HashMap<String, Value>> =
                    session.messages[session.last_consolidated..].to_vec();
                session.clear();
                let session_clone = session.clone();
                sessions.save(&session_clone);
                sessions.invalidate(&session_key);

                // Archive snapshot in background (fire and forget)
                if !snapshot.is_empty() {
                    let provider = self.provider.clone();
                    let model = self.model.clone();
                    let workspace = self.workspace.clone();
                    let ctx_window = self.context_window_tokens;
                    tokio::spawn(async move {
                        let mut mem = MemoryConsolidator::new(&workspace, ctx_window);
                        mem.archive_messages(&snapshot, provider.as_ref(), &model).await;
                    });
                }

                return Some(OutboundMessage::new(&msg.channel, &msg.chat_id, "New session started."));
            }
            "/status" => {
                let mut sessions = self.sessions.lock().await;
                let session = sessions.get_or_create(&session_key);
                let session_clone = session.clone();
                drop(sessions);
                let content = self.build_status_content(&session_clone, 0).await;
                return Some(OutboundMessage::new(&msg.channel, &msg.chat_id, &content));
            }
            "/help" => {
                let lines = vec![
                    "🐈 ombudsman commands:",
                    "/new — Start a new conversation",
                    "/stop — Stop the current task",
                    "/status — Show bot status",
                    "/help — Show available commands",
                ];
                return Some(OutboundMessage {
                    channel: msg.channel.clone(),
                    chat_id: msg.chat_id.clone(),
                    content: lines.join("\n"),
                    reply_to: None,
                    media: Vec::new(),
                    metadata: {
                        let mut m = HashMap::new();
                        m.insert("render_as".to_string(), json!("text"));
                        m
                    },
                });
            }
            _ => {}
        }

        // Lazily connect to MCP servers on first message
        self.connect_mcp().await;

        // Update per-message context for SpawnTool and MessageTool
        {
            let mut spawn_ctx = self.spawn_ctx.lock().unwrap();
            spawn_ctx.channel = msg.channel.clone();
            spawn_ctx.chat_id = msg.chat_id.clone();
            spawn_ctx.session_key = session_key.clone();
        }
        {
            let mut msg_ctx = self.message_ctx.lock().unwrap();
            msg_ctx.channel = msg.channel.clone();
            msg_ctx.chat_id = msg.chat_id.clone();
        }

        // Consolidate memory if needed
        {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_or_create(&session_key);
            let provider = self.provider.clone();
            let model = self.model.clone();
            let mut mem = self.memory.lock().await;
            mem.maybe_consolidate_by_tokens(session, provider.as_ref(), &model).await;
        }

        // Build messages
        let (history, history_len) = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_or_create(&session_key);
            let history = session.get_history(0);
            let len = history.len();
            (history, len)
        };

        let initial_messages = self.context.build_messages(
            &history,
            &msg.content,
            if msg.media.is_empty() { None } else { Some(&msg.media) },
            Some(&msg.channel),
            Some(&msg.chat_id),
            "user",
        );

        let bus_clone = self.bus.clone();
        let channel = msg.channel.clone();
        let chat_id = msg.chat_id.clone();
        let meta = msg.metadata.clone();
        let send_progress = self.channels_config.send_progress;
        let send_hints = self.channels_config.send_tool_hints;

        let progress_cb: Arc<dyn Fn(String, bool) + Send + Sync> =
            Arc::new(move |content: String, tool_hint: bool| {
                if !send_progress {
                    return;
                }
                if tool_hint && !send_hints {
                    return;
                }
                let mut m = meta.clone();
                m.insert("_progress".to_string(), json!(true));
                m.insert("_tool_hint".to_string(), json!(tool_hint));
                let out = OutboundMessage {
                    channel: channel.clone(),
                    chat_id: chat_id.clone(),
                    content,
                    reply_to: None,
                    media: Vec::new(),
                    metadata: m,
                };
                let bus = bus_clone.clone();
                tokio::spawn(async move {
                    let _ = bus.publish_outbound(out).await;
                });
            });

        let (final_content, _, all_msgs) = self
            .run_agent_loop(initial_messages, Some(on_progress.unwrap_or(progress_cb)))
            .await;

        let final_content =
            final_content.unwrap_or_else(|| "I've completed processing but have no response to give.".to_string());

        // Save turn
        {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_or_create(&session_key);
            let skip = 1 + history_len; // skip system prompt + history
            self.save_turn(session, &all_msgs, skip);
            let session_clone = session.clone();
            sessions.save(&session_clone);
        }

        // Trigger background consolidation
        {
            let provider = self.provider.clone();
            let model = self.model.clone();
            let workspace = self.workspace.clone();
            let ctx_window = self.context_window_tokens;
            let session_key2 = session_key.clone();
            if let Ok(mut sm) = SessionManager::new(&workspace) {
                let session_copy = sm.get_or_create(&session_key2).clone();
                tokio::spawn(async move {
                    let mut mem = MemoryConsolidator::new(&workspace, ctx_window);
                    let mut session_copy = session_copy;
                    mem.maybe_consolidate_by_tokens(&mut session_copy, provider.as_ref(), &model).await;
                });
            }
        }

        let preview = if final_content.len() > 120 {
            format!("{}...", &final_content[..120])
        } else {
            final_content.clone()
        };
        info!("Response to {}:{}: {}", msg.channel, msg.sender_id, preview);

        Some(OutboundMessage {
            channel: msg.channel.clone(),
            chat_id: msg.chat_id.clone(),
            content: final_content,
            reply_to: None,
            media: Vec::new(),
            metadata: msg.metadata.clone(),
        })
    }

    /// Run the agent, consuming messages from the bus.
    pub async fn run(&self) {
        *self.running.lock().await = true;
        info!("Agent loop started");

        loop {
            if !*self.running.lock().await {
                break;
            }

            let msg = match tokio::time::timeout(
                tokio::time::Duration::from_secs(1),
                self.bus.consume_inbound(),
            )
            .await
            {
                Ok(Some(msg)) => msg,
                Ok(None) | Err(_) => continue,
            };

            let cmd = msg.content.trim().to_lowercase();
            if cmd == "/stop" {
                let content = "No active task to stop.";
                let out = OutboundMessage::new(&msg.channel, &msg.chat_id, content);
                let _ = self.bus.publish_outbound(out).await;
                continue;
            }

            if let Some(response) = self.process_message(&msg, None).await {
                let _ = self.bus.publish_outbound(response).await;
            }
        }
    }

    pub async fn stop(&self) {
        *self.running.lock().await = false;
        info!("Agent loop stopping");
    }

    /// Process a message directly (for CLI use).
    pub async fn process_direct(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
        on_progress: Option<Arc<dyn Fn(String, bool) + Send + Sync>>,
    ) -> Option<OutboundMessage> {
        let msg = InboundMessage::new(channel, "user", chat_id, content);
        let msg = InboundMessage {
            session_key_override: Some(session_key.to_string()),
            ..msg
        };
        self.process_message(&msg, on_progress).await
    }
}
