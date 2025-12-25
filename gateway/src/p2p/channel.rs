//! WebRTC Data Channel implementation

use super::P2PError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Messages that can be sent over a data channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChannelMessage {
    /// Raw binary data
    Binary(Vec<u8>),

    /// Text message
    Text(String),

    /// Ping for keep-alive
    Ping,

    /// Pong response to ping
    Pong,

    /// Custom message type with label
    Custom {
        label: String,
        data: Vec<u8>,
    },
}

/// State of a data channel
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChannelState {
    Connecting,
    Open,
    Closing,
    Closed,
}

/// WebRTC Data Channel for peer-to-peer data transfer
#[derive(Clone)]
pub struct DataChannel {
    label: String,
    state: Arc<RwLock<ChannelState>>,
    send_tx: Arc<RwLock<Option<mpsc::Sender<Vec<u8>>>>>,
    recv_rx: Arc<RwLock<Option<mpsc::Receiver<Vec<u8>>>>>,
    ordered: bool,
    max_retransmits: Option<u16>,
    max_packet_life_time: Option<u16>,
}

impl DataChannel {
    /// Create a new data channel with the given label
    pub fn new(label: String) -> Self {
        let (send_tx, _send_rx) = mpsc::channel::<Vec<u8>>(1000);
        let (recv_tx, recv_rx) = mpsc::channel::<Vec<u8>>(1000);

        // Spawn a task to handle the mock channel
        let recv_tx = recv_tx;
        tokio::spawn(async move {
            // In production, this would handle actual WebRTC data channel events
            let _ = recv_tx;
        });

        Self {
            label,
            state: Arc::new(RwLock::new(ChannelState::Open)),
            send_tx: Arc::new(RwLock::new(Some(send_tx))),
            recv_rx: Arc::new(RwLock::new(Some(recv_rx))),
            ordered: true,
            max_retransmits: None,
            max_packet_life_time: None,
        }
    }

    /// Create a data channel with specific options
    pub fn with_options(
        label: String,
        ordered: bool,
        max_retransmits: Option<u16>,
        max_packet_life_time: Option<u16>,
    ) -> Self {
        let mut channel = Self::new(label);
        channel.ordered = ordered;
        channel.max_retransmits = max_retransmits;
        channel.max_packet_life_time = max_packet_life_time;
        channel
    }

    /// Get the channel label
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Get the current channel state
    pub async fn state(&self) -> ChannelState {
        *self.state.read().await
    }

    /// Check if the channel is open
    pub async fn is_open(&self) -> bool {
        *self.state.read().await == ChannelState::Open
    }

    /// Send data over the channel
    pub async fn send(&self, data: &[u8]) -> Result<(), P2PError> {
        let state = *self.state.read().await;
        if state != ChannelState::Open {
            return Err(P2PError::Channel(
                format!("Cannot send: channel state is {:?}", state)
            ));
        }

        if let Some(ref tx) = *self.send_tx.read().await {
            tx.send(data.to_vec()).await
                .map_err(|e| P2PError::Channel(format!("Send failed: {}", e)))?;

            tracing::debug!("Sent {} bytes on channel '{}'", data.len(), self.label);
        }

        Ok(())
    }

    /// Send a message over the channel
    pub async fn send_message(&self, message: ChannelMessage) -> Result<(), P2PError> {
        let data = serde_json::to_vec(&message)
            .map_err(|e| P2PError::Channel(format!("Serialization failed: {}", e)))?;

        self.send(&data).await
    }

    /// Receive data from the channel (non-blocking)
    pub async fn try_receive(&self) -> Result<Option<Vec<u8>>, P2PError> {
        let mut recv_rx = self.recv_rx.write().await;

        if let Some(ref mut rx) = *recv_rx {
            match rx.try_recv() {
                Ok(data) => Ok(Some(data)),
                Err(mpsc::error::TryRecvError::Empty) => Ok(None),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    *self.state.write().await = ChannelState::Closed;
                    Err(P2PError::Channel("Channel disconnected".to_string()))
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Receive a message from the channel (non-blocking)
    pub async fn try_receive_message(&self) -> Result<Option<ChannelMessage>, P2PError> {
        if let Some(data) = self.try_receive().await? {
            let message: ChannelMessage = serde_json::from_slice(&data)
                .map_err(|e| P2PError::Channel(format!("Deserialization failed: {}", e)))?;
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    /// Close the data channel
    pub async fn close(&self) -> Result<(), P2PError> {
        *self.state.write().await = ChannelState::Closing;

        // Drop the sender to signal close
        *self.send_tx.write().await = None;

        *self.state.write().await = ChannelState::Closed;

        tracing::debug!("Data channel '{}' closed", self.label);

        Ok(())
    }

    /// Get channel statistics
    pub async fn stats(&self) -> ChannelStats {
        ChannelStats {
            label: self.label.clone(),
            state: *self.state.read().await,
            ordered: self.ordered,
            max_retransmits: self.max_retransmits,
            max_packet_life_time: self.max_packet_life_time,
        }
    }
}

/// Statistics for a data channel
#[derive(Clone, Debug)]
pub struct ChannelStats {
    pub label: String,
    pub state: ChannelState,
    pub ordered: bool,
    pub max_retransmits: Option<u16>,
    pub max_packet_life_time: Option<u16>,
}
