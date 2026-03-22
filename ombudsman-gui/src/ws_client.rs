//! WebSocket subscription for the iced GUI.
//!
//! Pattern: `Subscription::run_with(url, ws_stream)` — the subscription
//! connects, sends a `WsEvent::Ready(sender)` back so the app can push
//! `ClientMsg` values, then streams `WsEvent::ServerMessage` events.

use futures::stream::BoxStream;
use futures::{SinkExt, StreamExt};
use iced::stream;
use ombudsman_core::protocol::{ClientMsg, ServerMsg, decode_server_msg, encode_client_msg};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsTxt};

/// Events produced by the WS subscription.
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// Subscription connected; the `Sender` lets the app push `ClientMsg`s.
    Ready(futures::channel::mpsc::Sender<ClientMsg>),
    /// A message arrived from the server.
    ServerMessage(ServerMsg),
    /// The connection was lost or failed to establish.
    Disconnected(String),
}

/// Build the `BoxStream<WsEvent>` that backs the `Subscription`.
///
/// Signature matches `fn(&String) -> BoxStream<WsEvent>` so it can be used
/// as the `builder` argument to `Subscription::run_with`.
pub fn ws_stream(url: &String) -> futures::stream::BoxStream<'static, WsEvent> {
    let url = url.clone();

    Box::pin(stream::channel(
        32,
        async move |mut output: futures::channel::mpsc::Sender<WsEvent>| {
            // Establish WS connection.
            let ws_stream = match connect_async(&url).await {
                Ok((s, _)) => s,
                Err(e) => {
                    let _ = output
                        .send(WsEvent::Disconnected(format!("Connect: {}", e)))
                        .await;
                    return;
                }
            };

            let (mut sink, mut recv) = ws_stream.split();

            // Bidirectional channel: app → subscription → WS sink.
            let (app_tx, mut app_rx) = futures::channel::mpsc::channel::<ClientMsg>(32);

            // Notify application that the subscription is ready.
            if output.send(WsEvent::Ready(app_tx)).await.is_err() {
                return;
            }

            // Event loop.
            loop {
                // Use tokio::select! (available because iced enables the tokio feature).
                tokio::select! {
                    // Outbound: app message → WS sink.
                    msg = app_rx.select_next_some() => {
                        match encode_client_msg(&msg) {
                            Ok(bytes) => {
                                if sink.send(WsTxt::Binary(bytes.into())).await.is_err() {
                                    let _ = output
                                        .send(WsEvent::Disconnected("Send failed".to_string()))
                                        .await;
                                    return;
                                }
                            }
                            Err(e) => {
                                let _ = output
                                    .send(WsEvent::Disconnected(format!("Encode: {}", e)))
                                    .await;
                                return;
                            }
                        }
                    }
                    // Inbound: WS frame → app.
                    frame = recv.next() => {
                        match frame {
                            Some(Ok(WsTxt::Binary(b))) => match decode_server_msg(&b) {
                                Ok(msg) => {
                                    if output.send(WsEvent::ServerMessage(msg)).await.is_err() {
                                        return;
                                    }
                                }
                                Err(e) => {
                                    let _ = output
                                        .send(WsEvent::Disconnected(format!("Decode: {}", e)))
                                        .await;
                                    return;
                                }
                            },
                            Some(Ok(WsTxt::Close(_))) | Some(Err(_)) | None => {
                                let _ = output
                                    .send(WsEvent::Disconnected("Connection closed".to_string()))
                                    .await;
                                return;
                            }
                            Some(Ok(_)) => {} // ping/pong/text — ignore
                        }
                    }
                }
            }
        },
    ))
}
