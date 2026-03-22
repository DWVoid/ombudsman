//! Tool registry for managing and executing tools.

use crate::agent::tools::base::Tool;
use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

/// Registry of all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Clone this registry into a new, independent registry.
    pub fn clone_registry(&self) -> ToolRegistry {
        let mut new_reg = ToolRegistry::new();
        for (name, tool) in &self.tools {
            new_reg.tools.insert(name.clone(), tool.clone_box());
        }
        new_reg
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Get all registered tool names, sorted.
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get all tool definitions in OpenAI function calling format.
    pub fn get_definitions(&self) -> Vec<Value> {
        let mut defs: Vec<Value> = self.tools.values().map(|t| t.to_schema()).collect();
        defs.sort_by(|a, b| {
            let an = a["function"]["name"].as_str().unwrap_or("");
            let bn = b["function"]["name"].as_str().unwrap_or("");
            an.cmp(bn)
        });
        defs
    }

    /// Execute a tool by name with the given arguments.
    pub async fn execute(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
    ) -> String {
        match self.tools.get(name) {
            Some(tool) => {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(300),
                    tool.execute(args),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => format!("Error: Tool '{}' timed out", name),
                }
            }
            None => {
                warn!("Unknown tool: {}", name);
                format!("Error: Unknown tool '{}'", name)
            }
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
