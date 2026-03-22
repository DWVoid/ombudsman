//! Session management for conversation history.

use anyhow::Result;
use chrono::{DateTime, Local};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

/// A conversation session.
#[derive(Debug, Clone)]
pub struct Session {
    pub key: String,
    pub messages: Vec<HashMap<String, Value>>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub metadata: HashMap<String, Value>,
    /// Number of messages already consolidated to memory files.
    pub last_consolidated: usize,
}

impl Session {
    pub fn new(key: &str) -> Self {
        let now = Local::now();
        Self {
            key: key.to_string(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
            last_consolidated: 0,
        }
    }

    /// Find first index where every tool result has a matching assistant tool_call.
    fn find_legal_start(messages: &[HashMap<String, Value>]) -> usize {
        let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut start = 0;
        for (i, msg) in messages.iter().enumerate() {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            if role == "assistant" {
                if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tcs {
                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                            declared.insert(id.to_string());
                        }
                    }
                }
            } else if role == "tool" {
                if let Some(tid) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                    if !declared.contains(tid) {
                        start = i + 1;
                        declared.clear();
                        // Re-scan declared from new start
                        for prev in &messages[start..=i.min(messages.len() - 1)] {
                            if prev.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                                if let Some(tcs) = prev.get("tool_calls").and_then(|v| v.as_array()) {
                                    for tc in tcs {
                                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                            declared.insert(id.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        start
    }

    /// Return unconsolidated messages for LLM input.
    pub fn get_history(&self, max_messages: usize) -> Vec<HashMap<String, Value>> {
        let unconsolidated = &self.messages[self.last_consolidated..];
        let sliced = if max_messages > 0 && unconsolidated.len() > max_messages {
            &unconsolidated[unconsolidated.len() - max_messages..]
        } else {
            unconsolidated
        };

        // Drop leading non-user messages
        let sliced = {
            let start = sliced
                .iter()
                .position(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
                .unwrap_or(0);
            &sliced[start..]
        };

        // Align to legal tool-call boundary
        let start = Self::find_legal_start(sliced);
        let sliced = &sliced[start..];

        sliced
            .iter()
            .map(|msg| {
                let mut entry = HashMap::new();
                entry.insert(
                    "role".to_string(),
                    msg.get("role").cloned().unwrap_or(Value::Null),
                );
                entry.insert(
                    "content".to_string(),
                    msg.get("content").cloned().unwrap_or(Value::String(String::new())),
                );
                for key in &["tool_calls", "tool_call_id", "name"] {
                    if let Some(v) = msg.get(*key) {
                        entry.insert(key.to_string(), v.clone());
                    }
                }
                entry
            })
            .collect()
    }

    /// Clear all messages and reset to initial state.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.last_consolidated = 0;
        self.updated_at = Local::now();
    }
}

/// Manages conversation sessions with JSONL file persistence.
pub struct SessionManager {
    workspace: PathBuf,
    sessions_dir: PathBuf,
    cache: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new(workspace: &Path) -> Result<Self> {
        let sessions_dir = workspace.join("sessions");
        std::fs::create_dir_all(&sessions_dir)?;
        Ok(Self {
            workspace: workspace.to_path_buf(),
            sessions_dir,
            cache: HashMap::new(),
        })
    }

    fn get_session_path(&self, key: &str) -> PathBuf {
        let safe_key = key.replace(':', "_").replace(['<', '>', '/', '\\', '|', '?', '*'], "_");
        self.sessions_dir.join(format!("{}.jsonl", safe_key))
    }

    /// Get an existing session or create a new one.
    pub fn get_or_create(&mut self, key: &str) -> &mut Session {
        if !self.cache.contains_key(key) {
            let session = self.load(key).unwrap_or_else(|| Session::new(key));
            self.cache.insert(key.to_string(), session);
        }
        self.cache.get_mut(key).unwrap()
    }

    fn load(&self, key: &str) -> Option<Session> {
        let path = self.get_session_path(key);
        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        let mut messages = Vec::new();
        let mut metadata = HashMap::new();
        let mut created_at = Local::now();
        let mut last_consolidated = 0usize;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let data: Value = match serde_json::from_str(line) {
                Ok(d) => d,
                Err(e) => {
                    warn!("Failed to parse session line: {}", e);
                    continue;
                }
            };

            if data.get("_type").and_then(|v| v.as_str()) == Some("metadata") {
                if let Some(m) = data.get("metadata").and_then(|v| v.as_object()) {
                    for (k, v) in m {
                        metadata.insert(k.clone(), v.clone());
                    }
                }
                if let Some(ts) = data.get("created_at").and_then(|v| v.as_str()) {
                    if let Ok(dt) = ts.parse::<DateTime<Local>>() {
                        created_at = dt;
                    }
                }
                last_consolidated = data
                    .get("last_consolidated")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
            } else {
                let mut msg = HashMap::new();
                if let Some(obj) = data.as_object() {
                    for (k, v) in obj {
                        msg.insert(k.clone(), v.clone());
                    }
                }
                if !msg.is_empty() {
                    messages.push(msg);
                }
            }
        }

        Some(Session {
            key: key.to_string(),
            messages,
            created_at,
            updated_at: Local::now(),
            metadata,
            last_consolidated,
        })
    }

    /// Save a session to disk.
    pub fn save(&self, session: &Session) {
        let path = self.get_session_path(&session.key);
        let mut lines = Vec::new();

        let metadata_line = serde_json::json!({
            "_type": "metadata",
            "key": session.key,
            "created_at": session.created_at.to_rfc3339(),
            "updated_at": session.updated_at.to_rfc3339(),
            "metadata": session.metadata,
            "last_consolidated": session.last_consolidated,
        });
        lines.push(serde_json::to_string(&metadata_line).unwrap_or_default());

        for msg in &session.messages {
            if let Ok(s) = serde_json::to_string(msg) {
                lines.push(s);
            }
        }

        let content = lines.join("\n") + "\n";
        if let Err(e) = std::fs::write(&path, content) {
            warn!("Failed to save session {}: {}", session.key, e);
        }
    }

    /// Invalidate a session from in-memory cache.
    pub fn invalidate(&mut self, key: &str) {
        self.cache.remove(key);
    }
}
