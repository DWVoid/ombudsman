//! Memory system for persistent agent memory.
//! Two-layer memory: MEMORY.md (long-term facts) + HISTORY.md (grep-searchable log).

use crate::providers::base::LLMProvider;
use crate::session::Session;
use chrono::Local;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const SAVE_MEMORY_TOOL: &str = r#"[
  {
    "type": "function",
    "function": {
      "name": "save_memory",
      "description": "Save the memory consolidation result to persistent storage.",
      "parameters": {
        "type": "object",
        "properties": {
          "history_entry": {
            "type": "string",
            "description": "A paragraph summarizing key events/decisions/topics. Start with [YYYY-MM-DD HH:MM]."
          },
          "memory_update": {
            "type": "string",
            "description": "Full updated long-term memory as markdown. Include all existing facts plus new ones."
          }
        },
        "required": ["history_entry", "memory_update"]
      }
    }
  }
]"#;

/// Two-layer memory store.
pub struct MemoryStore {
    memory_dir: PathBuf,
    memory_file: PathBuf,
    history_file: PathBuf,
    consecutive_failures: u32,
}

impl MemoryStore {
    const MAX_FAILURES: u32 = 3;

    pub fn new(workspace: &Path) -> Self {
        let memory_dir = workspace.join("memory");
        std::fs::create_dir_all(&memory_dir).ok();
        Self {
            memory_file: memory_dir.join("MEMORY.md"),
            history_file: memory_dir.join("HISTORY.md"),
            memory_dir,
            consecutive_failures: 0,
        }
    }

    pub fn read_long_term(&self) -> String {
        std::fs::read_to_string(&self.memory_file).unwrap_or_default()
    }

    pub fn write_long_term(&self, content: &str) {
        if let Err(e) = std::fs::write(&self.memory_file, content) {
            warn!("Failed to write long-term memory: {}", e);
        }
    }

    pub fn append_history(&self, entry: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.history_file)
            .unwrap();
        writeln!(file, "{}\n", entry.trim_end()).ok();
    }

    pub fn get_memory_context(&self) -> String {
        let long_term = self.read_long_term();
        if long_term.is_empty() {
            String::new()
        } else {
            format!("## Long-term Memory\n{}", long_term)
        }
    }

    fn format_messages(messages: &[HashMap<String, Value>]) -> String {
        let mut lines = Vec::new();
        for msg in messages {
            let content = msg.get("content");
            if content.is_none() || content == Some(&Value::Null) {
                continue;
            }
            let content_str = match content.unwrap() {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
            let ts = msg.get("timestamp").and_then(|v| v.as_str()).unwrap_or("?");
            let ts_short = &ts[..ts.len().min(16)];

            let tools = msg
                .get("tools_used")
                .and_then(|v| v.as_array())
                .map(|tools| {
                    let names: Vec<&str> = tools.iter().filter_map(|t| t.as_str()).collect();
                    format!(" [tools: {}]", names.join(", "))
                })
                .unwrap_or_default();

            lines.push(format!("[{}] {}{}: {}", ts_short, role.to_uppercase(), tools, content_str));
        }
        lines.join("\n")
    }

    pub async fn consolidate(
        &mut self,
        messages: &[HashMap<String, Value>],
        provider: &dyn LLMProvider,
        model: &str,
    ) -> bool {
        if messages.is_empty() {
            return true;
        }

        let current_memory = self.read_long_term();
        let formatted = Self::format_messages(messages);
        let prompt = format!(
            "Process this conversation and call the save_memory tool with your consolidation.\n\n## Current Long-term Memory\n{}\n\n## Conversation to Process\n{}",
            if current_memory.is_empty() { "(empty)".to_string() } else { current_memory.clone() },
            formatted
        );

        let chat_messages = vec![
            json!({
                "role": "system",
                "content": "You are a memory consolidation agent. Call the save_memory tool with your consolidation of the conversation."
            }),
            json!({"role": "user", "content": prompt}),
        ];

        let tools: Vec<Value> = serde_json::from_str(SAVE_MEMORY_TOOL).unwrap_or_default();
        let tool_choice = json!({"type": "function", "function": {"name": "save_memory"}});

        let response = provider
            .chat_with_retry(
                &chat_messages,
                Some(&tools),
                Some(model),
                Some(4096),
                Some(0.1),
                None,
                Some(&tool_choice),
            )
            .await;

        if !response.has_tool_calls() {
            warn!("Memory consolidation: LLM did not call save_memory");
            return self.fail_or_raw_archive(messages);
        }

        let tc = &response.tool_calls[0];
        let history_entry = match tc.arguments.get("history_entry").and_then(|v| v.as_str()) {
            Some(e) if !e.trim().is_empty() => e.to_string(),
            _ => {
                warn!("Memory consolidation: missing history_entry");
                return self.fail_or_raw_archive(messages);
            }
        };
        let memory_update = match tc.arguments.get("memory_update").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => {
                warn!("Memory consolidation: missing memory_update");
                return self.fail_or_raw_archive(messages);
            }
        };

        self.append_history(&history_entry);
        if memory_update != current_memory {
            self.write_long_term(&memory_update);
        }

        self.consecutive_failures = 0;
        info!("Memory consolidation done for {} messages", messages.len());
        true
    }

    fn fail_or_raw_archive(&mut self, messages: &[HashMap<String, Value>]) -> bool {
        self.consecutive_failures += 1;
        if self.consecutive_failures < Self::MAX_FAILURES {
            return false;
        }
        self.raw_archive(messages);
        self.consecutive_failures = 0;
        true
    }

    fn raw_archive(&self, messages: &[HashMap<String, Value>]) {
        let ts = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let formatted = Self::format_messages(messages);
        self.append_history(&format!(
            "[{}] [RAW] {} messages\n{}",
            ts,
            messages.len(),
            formatted
        ));
        warn!("Memory consolidation degraded: raw-archived {} messages", messages.len());
    }
}

/// Manages consolidation policy, locking, and session offset updates.
pub struct MemoryConsolidator {
    pub store: MemoryStore,
    context_window_tokens: u32,
    locks: HashMap<String, Arc<Mutex<()>>>,
}

impl MemoryConsolidator {
    const MAX_CONSOLIDATION_ROUNDS: u32 = 5;

    pub fn new(workspace: &Path, context_window_tokens: u32) -> Self {
        Self {
            store: MemoryStore::new(workspace),
            context_window_tokens,
            locks: HashMap::new(),
        }
    }

    fn get_lock(&mut self, session_key: &str) -> Arc<Mutex<()>> {
        self.locks
            .entry(session_key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Estimate token count for a session (approximate: 4 chars ≈ 1 token).
    fn estimate_tokens(messages: &[HashMap<String, Value>]) -> u32 {
        messages.iter().fold(0u32, |acc, msg| {
            let content_len = match msg.get("content") {
                Some(Value::String(s)) => s.len(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                    .map(|s| s.len())
                    .sum(),
                _ => 0,
            };
            acc + (content_len as u32 / 4 + 4)
        })
    }

    /// Maybe consolidate old messages if context window is too large.
    pub async fn maybe_consolidate_by_tokens(
        &mut self,
        session: &mut Session,
        provider: &dyn LLMProvider,
        model: &str,
    ) {
        if session.messages.is_empty() || self.context_window_tokens == 0 {
            return;
        }

        let lock = self.get_lock(&session.key);
        let _guard = lock.lock().await;

        let history = session.get_history(0);
        let estimated = Self::estimate_tokens(&history);

        if (estimated as u32) < self.context_window_tokens {
            debug!(
                "Token consolidation idle {}: {}/{}",
                session.key, estimated, self.context_window_tokens
            );
            return;
        }

        let target = self.context_window_tokens / 2;

        for round in 0..Self::MAX_CONSOLIDATION_ROUNDS {
            let history = session.get_history(0);
            let estimated = Self::estimate_tokens(&history);

            if (estimated as u32) <= target {
                return;
            }

            // Find a user-turn boundary
            let boundary = self.pick_boundary(session, estimated.saturating_sub(target) as usize);
            if let Some(end_idx) = boundary {
                let chunk: Vec<HashMap<String, Value>> =
                    session.messages[session.last_consolidated..end_idx].to_vec();
                if chunk.is_empty() {
                    return;
                }

                info!(
                    "Token consolidation round {} for {}: {}/{}, chunk={} msgs",
                    round,
                    session.key,
                    estimated,
                    self.context_window_tokens,
                    chunk.len()
                );

                if !self.store.consolidate(&chunk, provider, model).await {
                    return;
                }

                session.last_consolidated = end_idx;
            } else {
                debug!("Token consolidation: no safe boundary for {}", session.key);
                return;
            }
        }
    }

    fn pick_boundary(&self, session: &Session, tokens_to_remove: usize) -> Option<usize> {
        let start = session.last_consolidated;
        if start >= session.messages.len() {
            return None;
        }

        let mut removed = 0usize;
        let mut last_boundary = None;

        for (idx, msg) in session.messages[start..].iter().enumerate() {
            let abs_idx = start + idx;
            if abs_idx > start && msg.get("role").and_then(|v| v.as_str()) == Some("user") {
                last_boundary = Some((abs_idx, removed));
                if removed >= tokens_to_remove {
                    return Some(abs_idx);
                }
            }
            let content_len = match msg.get("content") {
                Some(Value::String(s)) => s.len(),
                _ => 0,
            };
            removed += content_len / 4 + 4;
        }

        last_boundary.map(|(idx, _)| idx)
    }

    pub async fn archive_messages(
        &mut self,
        messages: &[HashMap<String, Value>],
        provider: &dyn LLMProvider,
        model: &str,
    ) {
        if messages.is_empty() {
            return;
        }
        self.store.consolidate(messages, provider, model).await;
    }
}
