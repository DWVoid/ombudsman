//! Message bus event types.

use chrono::{DateTime, Local};
use std::collections::HashMap;

/// Message received from a chat channel (inbound to agent).
#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub channel: String,
    pub sender_id: String,
    pub chat_id: String,
    pub content: String,
    pub timestamp: DateTime<Local>,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub session_key_override: Option<String>,
}

impl InboundMessage {
    pub fn new(channel: &str, sender_id: &str, chat_id: &str, content: &str) -> Self {
        Self {
            channel: channel.to_string(),
            sender_id: sender_id.to_string(),
            chat_id: chat_id.to_string(),
            content: content.to_string(),
            timestamp: Local::now(),
            media: Vec::new(),
            metadata: HashMap::new(),
            session_key_override: None,
        }
    }

    /// The session key for this message.
    pub fn session_key(&self) -> String {
        self.session_key_override
            .clone()
            .unwrap_or_else(|| format!("{}:{}", self.channel, self.chat_id))
    }
}

/// Message to send to a chat channel (outbound from agent).
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl OutboundMessage {
    pub fn new(channel: &str, chat_id: &str, content: &str) -> Self {
        Self {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            content: content.to_string(),
            reply_to: None,
            media: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}
