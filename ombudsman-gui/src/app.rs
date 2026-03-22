//! iced `Application` implementation — the ViewModel layer of the MVVM pattern.
//!
//! # MVVM mapping
//! | MVVM          | iced concept                                |
//! |---------------|---------------------------------------------|
//! | ViewModel     | `AppState` + `update()`                     |
//! | View          | `view()` + `view/` modules                  |
//! | Model (remote)| server data arriving via WS subscription    |

use crate::viewmodel::{ChatMessage, ChatViewModel, ConnectionStatus, SettingsViewModel};
use crate::view::chat::chat_view;
use crate::view::settings_modal::settings_modal;
use crate::ws_client::{WsEvent, ws_stream};
use iced::{
    Alignment, Color, Element, Fill, Subscription, Task,
    widget::{button, column, container, row, stack, text},
};
use ombudsman_core::protocol::{ClientMsg, ServerMsg};

// ---------------------------------------------------------------------------
// Messages (events that drive state transitions)
// ---------------------------------------------------------------------------

/// All events the iced runtime delivers to `update()`.
#[derive(Debug, Clone)]
pub enum Message {
    // Chat panel
    InputChanged(String),
    SendMessage,

    // WebSocket events from the subscription
    WsEvent(WsEvent),

    // Settings modal
    OpenSettings,
    SettingsDraftServerUrl(String),
    SettingsDraftModel(String),
    SettingsDraftSessionKey(String),
    SettingsApply,
    SettingsCancel,
}

// ---------------------------------------------------------------------------
// AppState — ViewModel
// ---------------------------------------------------------------------------

/// Top-level application state (ViewModel).
pub struct AppState {
    pub chat: ChatViewModel,
    pub settings: SettingsViewModel,
    pub status: ConnectionStatus,

    /// Sender half of the WS subscription channel.  Set when the subscription
    /// reports `WsEvent::Ready`; `None` when disconnected.
    ws_sender: Option<futures::channel::mpsc::Sender<ClientMsg>>,
}

impl AppState {
    fn new() -> Self {
        let settings = SettingsViewModel::default();
        let chat = ChatViewModel::new(settings.session_key.clone());
        Self {
            chat,
            settings,
            status: ConnectionStatus::Connecting,
            ws_sender: None,
        }
    }

    /// Attempt to send a `ClientMsg` over the WS subscription channel.
    fn send_ws(&mut self, msg: ClientMsg) -> bool {
        if let Some(tx) = &mut self.ws_sender {
            if tx.try_send(msg).is_ok() {
                return true;
            }
            // Channel closed — clean up.
            self.ws_sender = None;
            self.status = ConnectionStatus::Disconnected;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// iced Application
// ---------------------------------------------------------------------------

pub struct App {
    state: AppState,
}

impl App {
    /// Entry point: build and run the iced application.
    pub fn run() -> iced::Result {
        iced::application(App::boot, App::update, App::view)
            .title("ombudsman")
            .subscription(App::subscription)
            .window_size(iced::Size::new(900.0, 680.0))
            .run()
    }

    /// Boot function — returns initial state.  The subscription will attempt
    /// to connect automatically on startup.
    fn boot() -> Self {
        App {
            state: AppState::new(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // ------------------------------------------------------------------
            // Chat
            // ------------------------------------------------------------------
            Message::InputChanged(s) => {
                self.state.chat.input = s;
                Task::none()
            }

            Message::SendMessage => {
                let text = self.state.chat.input.trim().to_string();
                if text.is_empty() || self.state.chat.is_loading {
                    return Task::none();
                }

                self.state.chat.begin_turn(text.clone());

                let msg = ClientMsg::Chat {
                    session_key: self.state.settings.session_key.clone(),
                    content: text,
                    media: vec![],
                };

                if !self.state.send_ws(msg) {
                    self.state.chat.push_message(ChatMessage::system(
                        "Not connected to server. Check Settings and try again.",
                    ));
                }

                Task::none()
            }

            // ------------------------------------------------------------------
            // WebSocket events from the subscription
            // ------------------------------------------------------------------
            Message::WsEvent(ev) => {
                self.handle_ws_event(ev);
                Task::none()
            }

            // ------------------------------------------------------------------
            // Settings
            // ------------------------------------------------------------------
            Message::OpenSettings => {
                self.state.settings.open();
                Task::none()
            }
            Message::SettingsDraftServerUrl(v) => {
                self.state.settings.draft_server_url = v;
                Task::none()
            }
            Message::SettingsDraftModel(v) => {
                self.state.settings.draft_model = v;
                Task::none()
            }
            Message::SettingsDraftSessionKey(v) => {
                self.state.settings.draft_session_key = v;
                Task::none()
            }
            Message::SettingsApply => {
                self.state.settings.apply();
                self.state.chat.session_key = self.state.settings.session_key.clone();
                // Dropping the sender forces the subscription to see a new URL
                // and reconnect (the subscription ID changes when url changes).
                self.state.ws_sender = None;
                self.state.status = ConnectionStatus::Connecting;
                Task::none()
            }
            Message::SettingsCancel => {
                self.state.settings.cancel();
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<Message> {
        let header = self.build_header();
        let body = chat_view(&self.state.chat, &self.state.status);

        let main = column![header, body]
            .width(Fill)
            .height(Fill);

        if let Some(modal) = settings_modal(&self.state.settings) {
            stack![main, modal].into()
        } else {
            main.into()
        }
    }

    /// Persistent WS subscription.  The subscription is identified by the
    /// server URL; changing it causes iced to restart the subscription.
    fn subscription(&self) -> Subscription<Message> {
        let url = self.state.settings.server_url.clone();
        Subscription::run_with(url, ws_stream)
            .map(Message::WsEvent)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn build_header(&self) -> Element<Message> {
        let title = text("🐈 ombudsman").size(20).color(Color::WHITE);

        let settings_btn = button(text("⚙ Settings").size(14).color(Color::WHITE))
            .on_press(Message::OpenSettings)
            .padding([8, 14])
            .style(|_theme, status| iced::widget::button::Style {
                background: Some(iced::Background::Color(match status {
                    iced::widget::button::Status::Hovered => Color::from_rgb(0.25, 0.45, 0.75),
                    _ => Color::from_rgb(0.18, 0.36, 0.64),
                })),
                text_color: Color::WHITE,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let header_row = row![title, settings_btn]
            .spacing(16)
            .align_y(Alignment::Center)
            .padding([0, 12]);

        container(header_row)
            .width(Fill)
            .padding([12, 16])
            .style(|_theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.12, 0.26, 0.52))),
                ..Default::default()
            })
            .into()
    }

    fn handle_ws_event(&mut self, ev: WsEvent) {
        match ev {
            WsEvent::Ready(sender) => {
                self.state.ws_sender = Some(sender);
                self.state.status = ConnectionStatus::Connected;
            }
            WsEvent::Disconnected(reason) => {
                self.state.ws_sender = None;
                self.state.status = ConnectionStatus::Error(reason.clone());
                if self.state.chat.is_loading {
                    self.state.chat.push_message(ChatMessage::system(
                        format!("Disconnected: {}", reason),
                    ));
                }
            }
            WsEvent::ServerMessage(msg) => match msg {
                ServerMsg::Response { content, .. } => {
                    self.state.chat.push_message(ChatMessage::assistant(content));
                }
                ServerMsg::Progress {
                    content,
                    is_tool_hint,
                } => {
                    if is_tool_hint {
                        self.state.chat.progress_text = Some(format!("⚙ {}", content));
                    } else {
                        self.state.chat.progress_text = Some(content);
                    }
                }
                ServerMsg::Error { message } => {
                    self.state.chat.push_message(ChatMessage::system(
                        format!("Server error: {}", message),
                    ));
                    self.state.chat.is_loading = false;
                }
                ServerMsg::Ack { .. } => {
                    // Acknowledged — nothing to show.
                }
            },
        }
    }
}
