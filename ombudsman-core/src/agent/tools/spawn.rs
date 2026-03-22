//! Spawn tool: creates a background subagent for a task.

use crate::agent::subagent::SubagentManager;
use crate::agent::tools::base::Tool;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mutable context updated per-message by the AgentLoop.
#[derive(Clone, Default)]
pub struct SpawnToolContext {
    pub channel: String,
    pub chat_id: String,
    pub session_key: String,
}

/// Tool to spawn a background subagent.
#[derive(Clone)]
pub struct SpawnTool {
    manager: Arc<SubagentManager>,
    context: Arc<Mutex<SpawnToolContext>>,
}

impl SpawnTool {
    pub fn new(manager: Arc<SubagentManager>, context: Arc<Mutex<SpawnToolContext>>) -> Self {
        Self { manager, context }
    }

    /// Return a shared handle to the context so the AgentLoop can update it.
    pub fn context_handle(&self) -> Arc<Mutex<SpawnToolContext>> {
        Arc::clone(&self.context)
    }
}

#[async_trait]
impl Tool for SpawnTool {
    fn name(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Spawn a subagent to handle a task in the background. \
         Use this for complex or time-consuming tasks that can run independently. \
         The subagent will complete the task and report back when done. \
         For deliverables or existing projects, inspect the workspace first \
         and use a dedicated subdirectory when helpful."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task for the subagent to complete"
                },
                "label": {
                    "type": "string",
                    "description": "Optional short label for the task (for display)"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return "Error: missing 'task' argument".to_string(),
        };
        let label = args
            .get("label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let (channel, chat_id, session_key) = {
            let ctx = self.context.lock().unwrap();
            (ctx.channel.clone(), ctx.chat_id.clone(), ctx.session_key.clone())
        };

        self.manager
            .spawn(task, label, channel, chat_id, session_key)
            .await
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}
