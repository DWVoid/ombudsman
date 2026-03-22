//! Message tool: sends a message to a chat channel via the outbound bus.

use crate::agent::tools::base::Tool;
use crate::bus::OutboundMessage;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Mutable context updated per-message by the AgentLoop.
#[derive(Clone, Default)]
pub struct MessageToolContext {
    pub channel: String,
    pub chat_id: String,
}

/// Tool to send messages to users on chat channels.
#[derive(Clone)]
pub struct MessageTool {
    outbound_tx: mpsc::Sender<OutboundMessage>,
    context: Arc<Mutex<MessageToolContext>>,
}

impl MessageTool {
    pub fn new(
        outbound_tx: mpsc::Sender<OutboundMessage>,
        context: Arc<Mutex<MessageToolContext>>,
    ) -> Self {
        Self { outbound_tx, context }
    }

    /// Return a shared handle to the context so the AgentLoop can update it.
    pub fn context_handle(&self) -> Arc<Mutex<MessageToolContext>> {
        Arc::clone(&self.context)
    }
}

#[async_trait]
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to the user on a chat channel. Use this when you want to \
         proactively push content to the user, or target a specific channel/chat_id \
         that differs from the current conversation."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content to send"
                },
                "channel": {
                    "type": "string",
                    "description": "Optional: target channel (e.g. telegram, discord). Defaults to current channel."
                },
                "chat_id": {
                    "type": "string",
                    "description": "Optional: target chat/user ID. Defaults to current chat."
                },
                "media": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional: list of file paths to attach"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return "Error: missing 'content' argument".to_string(),
        };

        let (default_channel, default_chat_id) = {
            let ctx = self.context.lock().unwrap();
            (ctx.channel.clone(), ctx.chat_id.clone())
        };

        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_channel)
            .to_string();

        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_chat_id)
            .to_string();

        if channel.is_empty() || chat_id.is_empty() {
            return "Error: No target channel/chat specified".to_string();
        }

        let media: Vec<String> = args
            .get("media")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let mut msg = OutboundMessage::new(&channel, &chat_id, &content);
        msg.media = media.clone();

        match self.outbound_tx.try_send(msg) {
            Ok(_) => {
                if media.is_empty() {
                    format!("Message sent to {}:{}", channel, chat_id)
                } else {
                    format!(
                        "Message sent to {}:{} with {} attachment(s)",
                        channel,
                        chat_id,
                        media.len()
                    )
                }
            }
            Err(e) => format!("Error sending message: {}", e),
        }
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}
