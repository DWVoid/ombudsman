//! WebSocket client abstraction for ombudsman-cli.
//!
//! Wraps the tokio-tungstenite connection with a simple send/recv API
//! over the ombudsman MessagePack wire protocol.

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use ombudsman_core::protocol::{ClientMsg, ServerMsg, decode_server_msg, encode_client_msg};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg, MaybeTlsStream, WebSocketStream};
use tokio::net::TcpStream;
use futures::stream::{SplitSink, SplitStream};

/// Events emitted by `WsClient::recv`.
#[derive(Debug)]
pub enum WsEvent {
    /// A decoded `ServerMsg` from the server.
    Message(ServerMsg),
    /// The connection closed or an error occurred.
    Disconnected(String),
}

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMsg>;
type WsStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// An active WebSocket connection to ombudsman-server.
pub struct WsClient {
    sink: WsSink,
    stream: WsStream,
}

impl WsClient {
    /// Connect to `url` and return a ready `WsClient`.
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws_stream, _) = connect_async(url).await
            .map_err(|e| anyhow::anyhow!("WebSocket connect to {} failed: {}", url, e))?;
        let (sink, stream) = ws_stream.split();
        Ok(Self { sink, stream })
    }

    /// Send a `ClientMsg` to the server.
    pub async fn send(&mut self, msg: ClientMsg) -> Result<()> {
        let bytes = encode_client_msg(&msg)?;
        self.sink.send(WsMsg::Binary(bytes.into())).await
            .map_err(|e| anyhow::anyhow!("WS send error: {}", e))
    }

    /// Receive the next `WsEvent`, blocking until one arrives.
    pub async fn recv(&mut self) -> Result<WsEvent> {
        match self.stream.next().await {
            Some(Ok(WsMsg::Binary(b))) => match decode_server_msg(&b) {
                Ok(msg) => Ok(WsEvent::Message(msg)),
                Err(e) => Ok(WsEvent::Disconnected(format!("Decode error: {}", e))),
            },
            Some(Ok(WsMsg::Close(_))) => Ok(WsEvent::Disconnected("Server closed connection".to_string())),
            Some(Ok(_)) => {
                // Ping/pong/text — skip and get next.
                Box::pin(self.recv()).await
            }
            Some(Err(e)) => Ok(WsEvent::Disconnected(format!("WS error: {}", e))),
            None => Ok(WsEvent::Disconnected("Connection ended".to_string())),
        }
    }

    /// Non-blocking attempt to receive a pending event.
    /// Returns `Ok(Some(event))` if one is available, `Ok(None)` if the channel
    /// is empty, or an `Err` on failure.
    pub async fn try_recv(&mut self) -> Result<Option<WsEvent>> {
        let timeout = tokio::time::Duration::from_millis(5);
        match tokio::time::timeout(timeout, self.recv()).await {
            Ok(result) => result.map(Some),
            Err(_) => Ok(None), // timed out — nothing pending
        }
    }
}
