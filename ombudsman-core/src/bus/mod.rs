//! Message bus for decoupling channels from the agent.

pub mod events;

pub use events::{InboundMessage, OutboundMessage};

use tokio::sync::mpsc;

/// Message bus using tokio MPSC channels.
pub struct MessageBus {
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: tokio::sync::Mutex<mpsc::Receiver<InboundMessage>>,
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: tokio::sync::Mutex<mpsc::Receiver<OutboundMessage>>,
}

impl MessageBus {
    /// Create a new message bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (in_tx, in_rx) = mpsc::channel(capacity);
        let (out_tx, out_rx) = mpsc::channel(capacity);
        Self {
            inbound_tx: in_tx,
            inbound_rx: tokio::sync::Mutex::new(in_rx),
            outbound_tx: out_tx,
            outbound_rx: tokio::sync::Mutex::new(out_rx),
        }
    }

    /// Send a message to the inbound queue (from channel → agent).
    pub async fn publish_inbound(&self, msg: InboundMessage) -> anyhow::Result<()> {
        self.inbound_tx.send(msg).await.map_err(|e| anyhow::anyhow!("Inbound send failed: {}", e))
    }

    /// Receive the next inbound message.
    pub async fn consume_inbound(&self) -> Option<InboundMessage> {
        self.inbound_rx.lock().await.recv().await
    }

    /// Send a message to the outbound queue (from agent → channel).
    pub async fn publish_outbound(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.outbound_tx.send(msg).await.map_err(|e| anyhow::anyhow!("Outbound send failed: {}", e))
    }

    /// Receive the next outbound message.
    pub async fn consume_outbound(&self) -> Option<OutboundMessage> {
        self.outbound_rx.lock().await.recv().await
    }

    /// Clone the inbound sender for use by channel implementations.
    pub fn inbound_sender(&self) -> mpsc::Sender<InboundMessage> {
        self.inbound_tx.clone()
    }

    /// Clone the outbound sender for use by the agent.
    pub fn outbound_sender(&self) -> mpsc::Sender<OutboundMessage> {
        self.outbound_tx.clone()
    }
}
