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
use ombudsman_core::{
    builder::build_agent_loop,
    bus::{InboundMessage, MessageBus},
    config::load_config,
    protocol::{ClientMsg, ServerMsg, decode_client_msg, encode_server_msg},
};
use std::sync::Arc;
use tracing::{error, info, warn};

/// Shared server state accessible from every WS handler.
#[derive(Clone)]
struct AppState {
    config: Arc<ombudsman_core::config::schema::Config>,
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
                    // Outbound bus messages are usually subagent completions
                    // re-broadcast to the user.
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
            _ => continue, // ignore text / ping / pong
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

        // Map ClientMsg → InboundMessage and dispatch.
        let (session_key, inbound) = match client_msg {
            ClientMsg::Chat {
                session_key,
                content,
                media,
            } => {
                let mut m = InboundMessage::new("ws", "user", "ws", &content);
                m.media = media;
                m.session_key_override = Some(session_key.clone());
                (session_key, m)
            }
            ClientMsg::Command {
                session_key,
                command,
            } => {
                let mut m = InboundMessage::new("ws", "user", "ws", &command);
                m.session_key_override = Some(session_key.clone());
                (session_key, m)
            }
        };

        // Ack immediately.
        let ack = ServerMsg::Ack {
            session_key: session_key.clone(),
        };
        if let Ok(b) = encode_server_msg(&ack) {
            let _ = server_tx.send(b).await;
        }

        // Run the agent, streaming progress events.
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

            if let Some(response) = agent_clone.process_message(&inbound, Some(on_progress)).await
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

    // Clean up background tasks.
    outbound_task.abort();
    subagent_task.abort();
    write_task.abort();

    info!("WebSocket session closed for {}", peer);
}
