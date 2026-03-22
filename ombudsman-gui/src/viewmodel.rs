//! Chat and settings view model types.
//!
//! These are the ViewModel layer of the MVVM pattern —
//! they hold all mutable UI-facing state and expose it to the view functions.

use serde::{Deserialize, Serialize};

/// Role of a chat message participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

/// A single message entry in the chat log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Optional tool-hint / progress snippet that led to this message.
    pub progress_hint: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            progress_hint: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            progress_hint: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
            progress_hint: None,
        }
    }
}

/// Mutable state for the chat panel ViewModel.
#[derive(Debug, Clone, Default)]
pub struct ChatViewModel {
    /// Full conversation history shown in the UI.
    pub messages: Vec<ChatMessage>,
    /// Current text in the input field.
    pub input: String,
    /// True while waiting for the server to respond.
    pub is_loading: bool,
    /// Latest progress/tool-hint text shown below the input.
    pub progress_text: Option<String>,
    /// Current session key sent to the server.
    pub session_key: String,
}

impl ChatViewModel {
    pub fn new(session_key: impl Into<String>) -> Self {
        Self {
            session_key: session_key.into(),
            ..Default::default()
        }
    }

    /// Push a message and clear loading state.
    pub fn push_message(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
        self.is_loading = false;
        self.progress_text = None;
    }

    /// Mark that we sent a user message; begin loading.
    pub fn begin_turn(&mut self, user_text: String) {
        self.messages.push(ChatMessage::user(user_text));
        self.is_loading = true;
        self.progress_text = None;
        self.input.clear();
    }
}

/// Mutable state for the settings modal ViewModel.
#[derive(Debug, Clone)]
pub struct SettingsViewModel {
    /// Whether the settings modal is currently visible.
    pub show: bool,
    /// WebSocket server URL.
    pub server_url: String,
    /// Model override (empty = use server default).
    pub model: String,
    /// Session identifier.
    pub session_key: String,
    /// Edited-but-not-yet-applied values.
    pub draft_server_url: String,
    pub draft_model: String,
    pub draft_session_key: String,
}

impl Default for SettingsViewModel {
    fn default() -> Self {
        Self {
            show: false,
            server_url: "ws://127.0.0.1:7878/ws".to_string(),
            model: String::new(),
            session_key: "gui:default".to_string(),
            draft_server_url: "ws://127.0.0.1:7878/ws".to_string(),
            draft_model: String::new(),
            draft_session_key: "gui:default".to_string(),
        }
    }
}

impl SettingsViewModel {
    /// Copy current values into draft so the user can edit without immediately committing.
    pub fn open(&mut self) {
        self.draft_server_url = self.server_url.clone();
        self.draft_model = self.model.clone();
        self.draft_session_key = self.session_key.clone();
        self.show = true;
    }

    /// Apply draft values and close the modal.
    pub fn apply(&mut self) {
        self.server_url = self.draft_server_url.trim().to_string();
        self.model = self.draft_model.trim().to_string();
        self.session_key = self.draft_session_key.trim().to_string();
        self.show = false;
    }

    /// Discard drafts and close the modal.
    pub fn cancel(&mut self) {
        self.show = false;
    }
}

/// WebSocket connection status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}

impl ConnectionStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting…",
            Self::Connected => "Connected",
            Self::Error(_) => "Error",
        }
    }
}
