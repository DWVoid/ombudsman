//! WebSocket server: bridges incoming WS connections to the AgentLoop.
//!
//! Protocol: each WebSocket frame is a MessagePack-encoded `ClientMsg` (binary from client)
//! or `ServerMsg` (binary from server).

use anyhow::Context as _;
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::get,
};
use futures::{SinkExt, StreamExt};
use ombudsman_core::protocol::{
    ClientMsg, ServerMsg, SessionInfo, decode_client_msg, encode_server_msg,
};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::builder::build_agent_loop;
use crate::bus::{InboundMessage, MessageBus};
use crate::config::load_config;

/// Shared server state accessible from every WS handler.
#[derive(Clone)]
struct AppState {
    config: Arc<crate::config::schema::Config>,
    model_override: Option<String>,
}

/// Start the axum WebSocket server and block until interrupted.
pub fn run_server(host: String, port: u16, model_override: Option<String>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    rt.block_on(async move {
        if let Err(e) = serve(host, port, model_override).await {
            error!("Server error: {}", e);
            std::process::exit(1);
        }
    });
}

async fn serve(host: String, port: u16, model_override: Option<String>) -> anyhow::Result<()> {
    let config = load_config().unwrap_or_default();

    let state = AppState {
        config: Arc::new(config),
        model_override,
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    info!("🐈 ombudsman WebSocket server listening on ws://{}/ws", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

/// Axum handler: upgrade HTTP → WebSocket, then spawn `handle_session`.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_session(socket, state))
}

/// Handle a single WebSocket session.
async fn handle_session(socket: WebSocket, state: AppState) {
    let peer = "ws-client";
    info!("New WebSocket connection from {}", peer);

    let bus = Arc::new(MessageBus::new(64));

    let agent = match build_agent_loop(
        bus.clone(),
        &state.config,
        state.model_override.clone(),
    ) {
        Ok(a) => a,
        Err(e) => {
            error!("Failed to build agent for WS session: {}", e);
            return;
        }
    };

    let (mut ws_sink, mut ws_stream) = socket.split();

    // Channel to forward ServerMsgs back to the WS client.
    let (server_tx, mut server_rx) =
        tokio::sync::mpsc::channel::<Vec<u8>>(32);

    // Task: drain the outbound bus and forward to the WS sink.
    let bus_out = Arc::clone(&bus);
    let server_tx_bus = server_tx.clone();
    let outbound_task = tokio::spawn(async move {
        loop {
            let msg = tokio::time::timeout(
                tokio::time::Duration::from_millis(100),
                bus_out.consume_outbound(),
            )
            .await;

            match msg {
                Ok(Some(out)) => {
                    let server_msg = ServerMsg::Response {
                        content: out.content,
                        media: out.media,
                    };
                    if let Ok(bytes) = encode_server_msg(&server_msg) {
                        let _ = server_tx_bus.send(bytes).await;
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }
    });

    // Task: poll the inbound bus for subagent results and re-process them.
    let agent_bg = Arc::clone(&agent);
    let bus_in = Arc::clone(&bus);
    let server_tx_subagent = server_tx.clone();
    let subagent_task = tokio::spawn(async move {
        loop {
            let maybe = tokio::time::timeout(
                tokio::time::Duration::from_millis(200),
                bus_in.consume_inbound(),
            )
            .await;

            let msg = match maybe {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(_) => continue,
            };

            if msg.sender_id != "subagent" {
                continue;
            }

            if let Some(response) = agent_bg.process_message(&msg, None).await {
                let server_msg = ServerMsg::Response {
                    content: response.content,
                    media: response.media,
                };
                if let Ok(bytes) = encode_server_msg(&server_msg) {
                    let _ = server_tx_subagent.send(bytes).await;
                }
            }
        }
    });

    // Task: forward encoded ServerMsgs to WS sink.
    let write_task = tokio::spawn(async move {
        while let Some(bytes) = server_rx.recv().await {
            if ws_sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        let _ = ws_sink.close().await;
    });

    // Main loop: receive ClientMsgs, dispatch to agent, stream back progress.
    while let Some(frame) = ws_stream.next().await {
        let frame = match frame {
            Ok(f) => f,
            Err(e) => {
                warn!("WS receive error: {}", e);
                break;
            }
        };

        let bytes = match frame {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            _ => continue,
        };

        let client_msg = match decode_client_msg(&bytes) {
            Ok(m) => m,
            Err(e) => {
                warn!("Failed to decode client message: {}", e);
                let err = ServerMsg::Error {
                    message: format!("Invalid message: {}", e),
                };
                if let Ok(b) = encode_server_msg(&err) {
                    let _ = server_tx.send(b).await;
                }
                continue;
            }
        };

        match client_msg {
            // ----------------------------------------------------------------
            // SessionList: enumerate all sessions from workspace
            // ----------------------------------------------------------------
            ClientMsg::SessionList => {
                let config = Arc::clone(&state.config);
                let tx = server_tx.clone();
                tokio::spawn(async move {
                    let workspace =
                        crate::config::paths::expand_path(&config.agents.defaults.workspace);
                    let sessions_dir = workspace.join("sessions");
                    let mut sessions: Vec<SessionInfo> = Vec::new();
                    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                                let key = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("")
                                    .replace('_', ":");
                                let meta = std::fs::metadata(&path).ok();
                                let updated_at = meta
                                    .and_then(|m| m.modified().ok())
                                    .and_then(|t| {
                                        t.duration_since(std::time::UNIX_EPOCH).ok()
                                    })
                                    .map(|d| {
                                        chrono::DateTime::<chrono::Local>::from(
                                            std::time::UNIX_EPOCH
                                                + std::time::Duration::from_secs(d.as_secs()),
                                        )
                                        .to_rfc3339()
                                    })
                                    .unwrap_or_default();
                                // Count messages by counting non-empty, non-metadata lines
                                let message_count = std::fs::read_to_string(&path)
                                    .unwrap_or_default()
                                    .lines()
                                    .filter(|l| {
                                        !l.trim().is_empty()
                                            && !l.contains(r#""_type":"metadata""#)
                                    })
                                    .count();
                                sessions.push(SessionInfo {
                                    key,
                                    message_count,
                                    updated_at,
                                });
                            }
                        }
                    }
                    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                    let msg = ServerMsg::SessionList { sessions };
                    if let Ok(b) = encode_server_msg(&msg) {
                        let _ = tx.send(b).await;
                    }
                });
            }

            // ----------------------------------------------------------------
            // Stop: signal the agent to stop the current task
            // ----------------------------------------------------------------
            ClientMsg::Stop { session_key } => {
                // Currently stop is a command the agent processes via "/stop"
                let mut m = InboundMessage::new("ws", "user", "ws", "/stop");
                m.session_key_override = Some(session_key.clone());
                let agent_clone = Arc::clone(&agent);
                let tx = server_tx.clone();
                tokio::spawn(async move {
                    if let Some(response) = agent_clone.process_message(&m, None).await {
                        let msg = ServerMsg::Stopped {
                            session_key: session_key.clone(),
                        };
                        if let Ok(b) = encode_server_msg(&msg) {
                            let _ = tx.send(b).await;
                        }
                        // Also send the agent's text response
                        let resp = ServerMsg::Response {
                            content: response.content,
                            media: response.media,
                        };
                        if let Ok(b) = encode_server_msg(&resp) {
                            let _ = tx.send(b).await;
                        }
                    }
                });
            }

            // ----------------------------------------------------------------
            // Chat / Command: dispatch to agent
            // ----------------------------------------------------------------
            ClientMsg::Chat {
                session_key,
                content,
                media,
            } => {
                let mut m = InboundMessage::new("ws", "user", "ws", &content);
                m.media = media;
                m.session_key_override = Some(session_key.clone());

                // Ack immediately.
                let ack = ServerMsg::Ack {
                    session_key: session_key.clone(),
                };
                if let Ok(b) = encode_server_msg(&ack) {
                    let _ = server_tx.send(b).await;
                }

                let agent_clone = Arc::clone(&agent);
                let tx_clone = server_tx.clone();
                tokio::spawn(async move {
                    let on_progress: Arc<dyn Fn(String, bool) + Send + Sync> = {
                        let tx = tx_clone.clone();
                        Arc::new(move |content: String, is_tool_hint: bool| {
                            let msg = ServerMsg::Progress {
                                content,
                                is_tool_hint,
                            };
                            if let Ok(b) = encode_server_msg(&msg) {
                                let inner_tx = tx.clone();
                                tokio::spawn(async move {
                                    let _ = inner_tx.send(b).await;
                                });
                            }
                        })
                    };

                    if let Some(response) =
                        agent_clone.process_message(&m, Some(on_progress)).await
                    {
                        let msg = ServerMsg::Response {
                            content: response.content,
                            media: response.media,
                        };
                        if let Ok(b) = encode_server_msg(&msg) {
                            let _ = tx_clone.send(b).await;
                        }
                    }
                });
            }

            ClientMsg::Command {
                session_key,
                command,
            } => {
                let mut m = InboundMessage::new("ws", "user", "ws", &command);
                m.session_key_override = Some(session_key.clone());

                // Ack immediately.
                let ack = ServerMsg::Ack {
                    session_key: session_key.clone(),
                };
                if let Ok(b) = encode_server_msg(&ack) {
                    let _ = server_tx.send(b).await;
                }

                let agent_clone = Arc::clone(&agent);
                let tx_clone = server_tx.clone();
                let session_key_clone = session_key.clone();
                tokio::spawn(async move {
                    if let Some(response) = agent_clone.process_message(&m, None).await {
                        // Detect /status response specially
                        let server_msg = if command.trim().to_lowercase() == "/status" {
                            ServerMsg::StatusResponse {
                                content: response.content,
                            }
                        } else {
                            ServerMsg::Response {
                                content: response.content,
                                media: response.media,
                            }
                        };
                        if let Ok(b) = encode_server_msg(&server_msg) {
                            let _ = tx_clone.send(b).await;
                        }
                        // If it was a /stop command, also send Stopped
                        if command.trim().to_lowercase() == "/stop" {
                            let stopped = ServerMsg::Stopped {
                                session_key: session_key_clone,
                            };
                            if let Ok(b) = encode_server_msg(&stopped) {
                                let _ = tx_clone.send(b).await;
                            }
                        }
                    }
                });
            }
        }
    }

    // Clean up background tasks.
    outbound_task.abort();
    subagent_task.abort();
    write_task.abort();

    info!("WebSocket session closed for {}", peer);
}
