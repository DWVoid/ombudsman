//! Shell execution tool.

use crate::agent::tools::base::Tool;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

pub struct ExecTool {
    timeout_secs: u64,
    working_dir: Option<String>,
    restrict_to_workspace: bool,
    path_append: String,
    deny_patterns: Vec<Regex>,
}

impl ExecTool {
    pub fn new(
        timeout_secs: u64,
        working_dir: Option<String>,
        restrict_to_workspace: bool,
        path_append: String,
    ) -> Self {
        let deny_patterns = vec![
            r"\brm\s+-[rf]{1,2}\b",
            r"\bdel\s+/[fq]\b",
            r"\brmdir\s+/s\b",
            r"(?:^|[;&|]\s*)format\b",
            r"\b(mkfs|diskpart)\b",
            r"\bdd\s+if=",
            r">\s*/dev/sd",
            r"\b(shutdown|reboot|poweroff)\b",
            r":\(\)\s*\{.*\};\s*:",
        ]
        .into_iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

        Self {
            timeout_secs,
            working_dir,
            restrict_to_workspace,
            path_append,
            deny_patterns,
        }
    }

    const MAX_OUTPUT: usize = 10_000;
    const MAX_TIMEOUT: u64 = 600;

    fn guard_command(&self, command: &str, _cwd: &str) -> Option<String> {
        let lower = command.to_lowercase();

        for pattern in &self.deny_patterns {
            if pattern.is_match(&lower) {
                return Some(
                    "Error: Command blocked by safety guard (dangerous pattern detected)".to_string(),
                );
            }
        }

        if self.restrict_to_workspace {
            if lower.contains("../") || lower.contains("..\\") {
                return Some(
                    "Error: Command blocked by safety guard (path traversal detected)".to_string(),
                );
            }
        }

        None
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &str { "exec" }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use with caution."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to execute"},
                "working_dir": {"type": "string", "description": "Optional working directory"},
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 60, max 600)",
                    "minimum": 1,
                    "maximum": 600
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return "Error: missing 'command' argument".to_string(),
        };

        let cwd = args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.working_dir.clone())
            .unwrap_or_else(|| ".".to_string());

        let timeout = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.timeout_secs)
            .min(Self::MAX_TIMEOUT);

        if let Some(err) = self.guard_command(&command, &cwd) {
            return err;
        }

        let mut env = std::collections::HashMap::new();
        for (k, v) in std::env::vars() {
            env.insert(k, v);
        }
        if !self.path_append.is_empty() {
            let path = env.get("PATH").cloned().unwrap_or_default();
            env.insert(
                "PATH".to_string(),
                format!("{}:{}", path, self.path_append),
            );
        }

        let child = match Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(&env)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("Error executing command: {}", e),
        };

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            child.wait_with_output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let mut parts = Vec::new();
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if !stdout.is_empty() {
                    parts.push(stdout);
                }
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if !stderr.trim().is_empty() {
                    parts.push(format!("STDERR:\n{}", stderr));
                }
                parts.push(format!("\nExit code: {}", output.status.code().unwrap_or(-1)));

                let result = parts.join("\n");

                // Truncate if too long (head + tail)
                if result.len() > Self::MAX_OUTPUT {
                    let half = Self::MAX_OUTPUT / 2;
                    format!(
                        "{}\n\n... ({} chars truncated) ...\n\n{}",
                        &result[..half],
                        result.len() - Self::MAX_OUTPUT,
                        &result[result.len() - half..]
                    )
                } else {
                    result
                }
            }
            Ok(Err(e)) => format!("Error executing command: {}", e),
            Err(_) => format!("Error: Command timed out after {} seconds", timeout),
        }
    }
}
