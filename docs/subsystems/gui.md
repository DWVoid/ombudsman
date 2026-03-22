# GUI Client

**Source:** `ombudsman-gui/`

`ombudsman-gui` is a desktop GUI client for `ombudsman-server`, built with the [iced](https://github.com/iced-rs/iced) GUI framework (v0.14).  It connects to the server via WebSocket using the shared `ombudsman-core` protocol, and follows an **MVVM** architecture.

---

## Table of Contents

1. [Architecture (MVVM)](#1-architecture-mvvm)
2. [Application Entry Point](#2-application-entry-point)
3. [AppState (ViewModel)](#3-appstate-viewmodel)
4. [ChatViewModel and SettingsViewModel](#4-chatviewmodel-and-settingsviewmodel)
5. [WebSocket Subscription](#5-websocket-subscription)
6. [Views](#6-views)
7. [Message Enum](#7-message-enum)
8. [Settings Modal](#8-settings-modal)

---

## 1. Architecture (MVVM)

| MVVM layer | iced concept |
|---|---|
| **Model** (remote) | Server state arriving via WebSocket (`ServerMsg` events) |
| **ViewModel** | `AppState` → `ChatViewModel` + `SettingsViewModel` |
| **View** | `view()` + `view/chat.rs` + `view/settings_modal.rs` |
| **Update** | `App::update(Message)` — pure state transitions |

iced drives the application as an elm-style message loop:

```
initial state (AppState::new)
      │
      ▼
   view()        ──── renders Element<Message>
      │
      ▼  user interaction or WsEvent
   update(Message) ─── returns new state + Task<Message>
      │
      └─ loop
```

---

## 2. Application Entry Point

**File:** `ombudsman-gui/src/main.rs` → `ombudsman-gui/src/app.rs`

```rust
pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title("ombudsman")
        .subscription(App::subscription)
        .window_size(iced::Size::new(900.0, 680.0))
        .run()
}
```

The application is registered as an iced application with a 900 × 680 default window.

---

## 3. AppState (ViewModel)

**File:** `ombudsman-gui/src/app.rs`

```rust
pub struct AppState {
    pub chat:     ChatViewModel,
    pub settings: SettingsViewModel,
    pub status:   ConnectionStatus,
    ws_sender:    Option<futures::channel::mpsc::Sender<ClientMsg>>,
}
```

`ws_sender` is populated when the WebSocket subscription fires `WsEvent::Ready`.  The app sends messages to the server by calling `try_send()` on this channel.

### ConnectionStatus

```rust
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}
```

Shown in the header bar.  Transitions are driven by `WsEvent::Ready` / `WsEvent::Disconnected`.

---

## 4. ChatViewModel and SettingsViewModel

**File:** `ombudsman-gui/src/viewmodel.rs`

### ChatViewModel

```rust
pub struct ChatViewModel {
    pub messages:      Vec<ChatMessage>,
    pub input:         String,
    pub is_loading:    bool,
    pub progress_text: Option<String>,
    pub session_key:   String,
}
```

`ChatMessage` entries carry a `ChatRole` (`User`, `Assistant`, `System`) and optional `progress_hint`.

### SettingsViewModel

Holds:
- `server_url` (default `ws://127.0.0.1:7878/ws`)
- `session_key` (default `gui:default`)
- `model` (optional — informational only; server uses its configured model)
- Draft fields (`draft_server_url`, `draft_session_key`, `draft_model`) for pending edits
- `is_open: bool` — whether the settings modal is visible

Applying settings persists the drafts to the live fields and drops the WS sender, which triggers an automatic reconnect (because the iced `Subscription` ID is keyed on the server URL).

---

## 5. WebSocket Subscription

**File:** `ombudsman-gui/src/ws_client.rs`

```rust
pub enum WsEvent {
    Ready(futures::channel::mpsc::Sender<ClientMsg>),
    ServerMessage(ServerMsg),
    Disconnected(String),
}

pub fn ws_stream(url: &String) -> BoxStream<'static, WsEvent>
```

The subscription is backed by `iced::stream::channel` + `tokio_tungstenite`.  On connection:
1. Sends `WsEvent::Ready(sender)` so `AppState` can hold the outbound channel.
2. Spawns an internal loop that forwards outbound `ClientMsg` from the channel → WS sink.
3. Streams inbound `ServerMsg` frames as `WsEvent::ServerMessage`.
4. Emits `WsEvent::Disconnected` on any error or close.

The subscription is registered with:

```rust
fn subscription(&self) -> Subscription<Message> {
    Subscription::run_with(self.state.settings.server_url.clone(), ws_stream)
        .map(Message::WsEvent)
}
```

Changing the server URL causes iced to cancel the old subscription and start a new one (automatic reconnect).

---

## 6. Views

**Files:** `ombudsman-gui/src/view/chat.rs`, `ombudsman-gui/src/view/settings_modal.rs`

### chat_view

Renders the main chat panel:
- Scrollable message list (user messages right-aligned, assistant and system left-aligned)
- Text input field
- Send button (also triggered by Enter key)
- Progress hint line below the input when `is_loading`

### Header

Built in `App::build_header()`:
- Logo + title on the left
- "⚙ Settings" button on the right (opens the settings modal)
- Blue gradient background bar

---

## 7. Message Enum

All user interactions and WebSocket events are expressed as variants of:

```rust
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
```

---

## 8. Settings Modal

The settings modal is rendered as an overlay using `iced::widget::stack`.  It is only shown when `SettingsViewModel::is_open` is `true`.  Fields:

| Field | Default | Description |
|---|---|---|
| Server URL | `ws://127.0.0.1:7878/ws` | WebSocket endpoint of `ombudsman-server` |
| Session key | `gui:default` | Session identifier sent with each `ClientMsg::Chat` |
| Model | _(empty)_ | Informational; the server uses its own configured model |

Pressing **Apply** closes the modal, persists the draft values, and triggers a WS reconnect if the server URL changed.  Pressing **Cancel** discards the draft.
