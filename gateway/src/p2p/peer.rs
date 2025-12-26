//! P2P Peer connection management with WebRTC

use super::P2PError;
use prost::bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

/// Configuration for a peer connection
#[derive(Clone, Debug, Default)]
pub struct PeerConfig {
    /// STUN server URLs
    pub stun_servers: Vec<String>,

    /// TURN server configurations
    pub turn_servers: Vec<TurnServer>,
}

/// TURN server configuration
#[derive(Clone, Debug)]
pub struct TurnServer {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}

/// Events that can occur during peer communication
#[derive(Clone, Debug)]
pub enum PeerEvent {
    /// Connection established
    Connected,

    /// Connection closed
    Disconnected,

    /// Data received from peer
    DataReceived(Vec<u8>),

    /// ICE candidate gathered
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },

    /// Error occurred
    Error(String),
}

/// Connection state of a peer
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

/// Represents a P2P peer connection using WebRTC
pub struct P2PPeer {
    remote_id: String,
    config: PeerConfig,
    peer_connection: Arc<RTCPeerConnection>,
    data_channel: Arc<RwLock<Option<Arc<RTCDataChannel>>>>,
    event_tx: Arc<RwLock<Option<mpsc::Sender<PeerEvent>>>>,
    ice_candidates: Arc<RwLock<Vec<RTCIceCandidateInit>>>,
}

impl P2PPeer {
    /// Maximum chunk size for DataChannel messages (16KB to be safe)
    pub const MAX_CHUNK_SIZE: usize = 16 * 1024;

    /// Create a new peer connection
    pub async fn new(remote_id: String, config: PeerConfig) -> Result<Self, P2PError> {
        let peer_connection = Self::create_peer_connection(&config).await?;

        Ok(Self {
            remote_id,
            config,
            peer_connection: Arc::new(peer_connection),
            data_channel: Arc::new(RwLock::new(None)),
            event_tx: Arc::new(RwLock::new(None)),
            ice_candidates: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Create the RTCPeerConnection with the given configuration
    async fn create_peer_connection(config: &PeerConfig) -> Result<RTCPeerConnection, P2PError> {
        // Create a MediaEngine (required even for data-only connections)
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()
            .map_err(|e| P2PError::Connection(format!("Failed to register codecs: {}", e)))?;

        // Create an interceptor registry
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)
            .map_err(|e| P2PError::Connection(format!("Failed to register interceptors: {}", e)))?;

        // Build the API
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        // Build ICE servers configuration
        let mut ice_servers = Vec::new();

        // Add STUN servers
        for url in &config.stun_servers {
            ice_servers.push(RTCIceServer {
                urls: vec![url.clone()],
                ..Default::default()
            });
        }

        // Add TURN servers
        for turn in &config.turn_servers {
            ice_servers.push(RTCIceServer {
                urls: turn.urls.clone(),
                username: turn.username.clone(),
                credential: turn.credential.clone(),
                ..Default::default()
            });
        }

        // If no servers configured, use default STUN
        if ice_servers.is_empty() {
            ice_servers.push(RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            });
        }

        let rtc_config = RTCConfiguration {
            ice_servers,
            ..Default::default()
        };

        let peer_connection = api.new_peer_connection(rtc_config).await
            .map_err(|e| P2PError::Connection(format!("Failed to create peer connection: {}", e)))?;

        Ok(peer_connection)
    }

    /// Get the remote peer ID
    pub fn remote_id(&self) -> &str {
        &self.remote_id
    }

    /// Get the current connection state
    pub fn state(&self) -> ConnectionState {
        match self.peer_connection.connection_state() {
            RTCPeerConnectionState::New => ConnectionState::New,
            RTCPeerConnectionState::Connecting => ConnectionState::Connecting,
            RTCPeerConnectionState::Connected => ConnectionState::Connected,
            RTCPeerConnectionState::Disconnected => ConnectionState::Disconnected,
            RTCPeerConnectionState::Failed => ConnectionState::Failed,
            RTCPeerConnectionState::Closed => ConnectionState::Disconnected,
            _ => ConnectionState::New,
        }
    }

    /// Subscribe to peer events
    pub async fn subscribe(&self) -> mpsc::Receiver<PeerEvent> {
        let (tx, rx) = mpsc::channel(100);
        *self.event_tx.write().await = Some(tx);
        rx
    }

    /// Set up event handlers for the peer connection
    pub async fn setup_handlers(&self) -> Result<(), P2PError> {
        let event_tx = self.event_tx.clone();
        let ice_candidates = self.ice_candidates.clone();

        // Handle ICE candidates
        self.peer_connection.on_ice_candidate(Box::new(move |candidate| {
            let event_tx = event_tx.clone();
            let ice_candidates = ice_candidates.clone();

            Box::pin(async move {
                if let Some(candidate) = candidate {
                    let candidate_json = match candidate.to_json() {
                        Ok(json) => json,
                        Err(e) => {
                            tracing::error!("Failed to serialize ICE candidate: {}", e);
                            return;
                        }
                    };

                    // Store the candidate
                    ice_candidates.write().await.push(RTCIceCandidateInit {
                        candidate: candidate_json.candidate.clone(),
                        sdp_mid: candidate_json.sdp_mid.clone(),
                        sdp_mline_index: candidate_json.sdp_mline_index,
                        ..Default::default()
                    });

                    // Notify via event
                    if let Some(ref tx) = *event_tx.read().await {
                        let _ = tx.send(PeerEvent::IceCandidate {
                            candidate: candidate_json.candidate,
                            sdp_mid: candidate_json.sdp_mid,
                            sdp_mline_index: candidate_json.sdp_mline_index,
                        }).await;
                    }
                }
            })
        }));

        // Handle connection state changes
        let event_tx = self.event_tx.clone();
        self.peer_connection.on_peer_connection_state_change(Box::new(move |state| {
            let event_tx = event_tx.clone();

            Box::pin(async move {
                tracing::info!("Peer connection state changed: {:?}", state);

                if let Some(ref tx) = *event_tx.read().await {
                    match state {
                        RTCPeerConnectionState::Connected => {
                            let _ = tx.send(PeerEvent::Connected).await;
                        }
                        RTCPeerConnectionState::Disconnected |
                        RTCPeerConnectionState::Failed |
                        RTCPeerConnectionState::Closed => {
                            let _ = tx.send(PeerEvent::Disconnected).await;
                        }
                        _ => {}
                    }
                }
            })
        }));

        Ok(())
    }

    /// Set up handlers for incoming data channels (for answerer)
    pub async fn setup_data_channel_handler(&self) -> Result<(), P2PError> {
        let data_channel_store = self.data_channel.clone();
        let event_tx = self.event_tx.clone();

        self.peer_connection.on_data_channel(Box::new(move |dc| {
            let data_channel_store = data_channel_store.clone();
            let event_tx = event_tx.clone();
            let dc_label = dc.label().to_string();

            Box::pin(async move {
                tracing::info!("New data channel: {}", dc_label);

                // Store the data channel
                *data_channel_store.write().await = Some(dc.clone());

                // Set up message handler
                let event_tx_msg = event_tx.clone();
                dc.on_message(Box::new(move |msg: DataChannelMessage| {
                    let event_tx = event_tx_msg.clone();
                    let data = msg.data.to_vec();

                    Box::pin(async move {
                        tracing::debug!("Received {} bytes on data channel", data.len());

                        if let Some(ref tx) = *event_tx.read().await {
                            let _ = tx.send(PeerEvent::DataReceived(data)).await;
                        }
                    })
                }));

                // Handle open event
                let event_tx_open = event_tx.clone();
                dc.on_open(Box::new(move || {
                    let event_tx = event_tx_open.clone();

                    Box::pin(async move {
                        tracing::info!("Data channel opened");

                        if let Some(ref tx) = *event_tx.read().await {
                            let _ = tx.send(PeerEvent::Connected).await;
                        }
                    })
                }));
            })
        }));

        Ok(())
    }

    /// Create an SDP offer for initiating a connection
    pub async fn create_offer(&self) -> Result<String, P2PError> {
        // Create a data channel first (offerer creates the channel)
        let dc = self.peer_connection.create_data_channel("data", None).await
            .map_err(|e| P2PError::Channel(format!("Failed to create data channel: {}", e)))?;

        *self.data_channel.write().await = Some(dc.clone());

        // Set up data channel handlers
        let event_tx = self.event_tx.clone();
        dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let event_tx = event_tx.clone();
            let data = msg.data.to_vec();

            Box::pin(async move {
                if let Some(ref tx) = *event_tx.read().await {
                    let _ = tx.send(PeerEvent::DataReceived(data)).await;
                }
            })
        }));

        // Create the offer
        let offer = self.peer_connection.create_offer(None).await
            .map_err(|e| P2PError::Connection(format!("Failed to create offer: {}", e)))?;

        // Set local description
        self.peer_connection.set_local_description(offer.clone()).await
            .map_err(|e| P2PError::Connection(format!("Failed to set local description: {}", e)))?;

        Ok(offer.sdp)
    }

    /// Create an SDP answer in response to an offer
    pub async fn create_answer(&self, offer_sdp: &str) -> Result<String, P2PError> {
        // Parse and set remote description (the offer)
        let offer = RTCSessionDescription::offer(offer_sdp.to_string())
            .map_err(|e| P2PError::Connection(format!("Failed to parse offer SDP: {}", e)))?;

        self.peer_connection.set_remote_description(offer).await
            .map_err(|e| P2PError::Connection(format!("Failed to set remote description: {}", e)))?;

        // Create the answer
        let answer = self.peer_connection.create_answer(None).await
            .map_err(|e| P2PError::Connection(format!("Failed to create answer: {}", e)))?;

        // Set local description
        self.peer_connection.set_local_description(answer.clone()).await
            .map_err(|e| P2PError::Connection(format!("Failed to set local description: {}", e)))?;

        tracing::info!("Created answer SDP");

        Ok(answer.sdp)
    }

    /// Set the remote SDP description (for the offerer receiving an answer)
    pub async fn set_remote_answer(&self, answer_sdp: &str) -> Result<(), P2PError> {
        let answer = RTCSessionDescription::answer(answer_sdp.to_string())
            .map_err(|e| P2PError::Connection(format!("Failed to parse answer SDP: {}", e)))?;

        self.peer_connection.set_remote_description(answer).await
            .map_err(|e| P2PError::Connection(format!("Failed to set remote description: {}", e)))?;

        Ok(())
    }

    /// Add an ICE candidate for NAT traversal
    pub async fn add_ice_candidate(
        &self,
        candidate: &str,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    ) -> Result<(), P2PError> {
        let candidate_init = RTCIceCandidateInit {
            candidate: candidate.to_string(),
            sdp_mid,
            sdp_mline_index,
            ..Default::default()
        };

        self.peer_connection.add_ice_candidate(candidate_init).await
            .map_err(|e| P2PError::Connection(format!("Failed to add ICE candidate: {}", e)))?;

        tracing::debug!("Added ICE candidate");

        Ok(())
    }

    /// Get gathered ICE candidates
    pub async fn get_ice_candidates(&self) -> Vec<RTCIceCandidateInit> {
        self.ice_candidates.read().await.clone()
    }

    /// Send data to the remote peer
    pub async fn send(&self, data: &[u8]) -> Result<(), P2PError> {
        let dc = self.data_channel.read().await;

        if let Some(ref channel) = *dc {
            channel.send(&Bytes::copy_from_slice(data)).await
                .map_err(|e| P2PError::Channel(format!("Failed to send data: {}", e)))?;

            tracing::debug!("Sent {} bytes", data.len());
        } else {
            return Err(P2PError::Channel("No data channel available".to_string()));
        }

        Ok(())
    }

    /// Close the peer connection

    /// Send data in chunks to avoid DataChannel message size limits
    ///
    /// For large responses (streaming), this splits the data into multiple messages.
    /// Each chunk is prefixed with a header indicating chunk index and total chunks.
    ///
    /// Chunk format:
    /// - chunk_index (4 bytes, big-endian u32)
    /// - total_chunks (4 bytes, big-endian u32)
    /// - is_last (1 byte, 0 or 1)
    /// - data (remaining bytes)
    pub async fn send_chunked(&self, data: &[u8]) -> Result<(), P2PError> {
        let dc = self.data_channel.read().await;

        if let Some(ref channel) = *dc {
            // Calculate chunk parameters
            let header_size = 9; // 4 + 4 + 1
            let payload_size = Self::MAX_CHUNK_SIZE - header_size;
            let total_chunks = (data.len() + payload_size - 1) / payload_size;
            let total_chunks = if total_chunks == 0 { 1 } else { total_chunks };

            tracing::debug!(
                "Sending {} bytes in {} chunks (payload_size={})",
                data.len(),
                total_chunks,
                payload_size
            );

            for (i, chunk_data) in data.chunks(payload_size).enumerate() {
                let is_last = i == total_chunks - 1;

                let mut chunk = Vec::with_capacity(header_size + chunk_data.len());
                chunk.extend_from_slice(&(i as u32).to_be_bytes());
                chunk.extend_from_slice(&(total_chunks as u32).to_be_bytes());
                chunk.push(if is_last { 1 } else { 0 });
                chunk.extend_from_slice(chunk_data);

                channel.send(&Bytes::copy_from_slice(&chunk)).await
                    .map_err(|e| P2PError::Channel(format!("Failed to send chunk {}/{}: {}", i + 1, total_chunks, e)))?;

                tracing::debug!("Sent chunk {}/{} ({} bytes)", i + 1, total_chunks, chunk.len());
            }
        } else {
            return Err(P2PError::Channel("No data channel available".to_string()));
        }

        Ok(())
    }

    /// Close the peer connection
    pub async fn close(&self) -> Result<(), P2PError> {
        self.peer_connection.close().await
            .map_err(|e| P2PError::Connection(format!("Failed to close connection: {}", e)))?;

        if let Some(ref tx) = *self.event_tx.read().await {
            let _ = tx.send(PeerEvent::Disconnected).await;
        }

        Ok(())
    }

    /// Get the peer connection for advanced operations
    pub fn peer_connection(&self) -> &Arc<RTCPeerConnection> {
        &self.peer_connection
    }
}
