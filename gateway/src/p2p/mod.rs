//! P2P Communication module using WebRTC
//!
//! This module provides peer-to-peer communication capabilities
//! for direct data transfer between gateway instances.
//!
//! ## Authentication
//!
//! P2P connections require authentication via the cf-wbrtc-auth server.
//! Use the `auth` module for OAuth setup and the `credentials` module
//! for managing API keys.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use gateway_lib::p2p::{auth, credentials, SignalingConfig, AuthenticatedSignalingClient};
//!
//! // Load or setup credentials
//! let creds = auth::load_or_setup(None, auth::SetupConfig {
//!     auth_server_url: "https://auth.example.com".to_string(),
//!     ..Default::default()
//! }).await?;
//!
//! // Create authenticated signaling client
//! let config = SignalingConfig {
//!     server_url: "wss://signaling.example.com/ws/app".to_string(),
//!     api_key: creds.api_key,
//!     app_name: "MyApp".to_string(),
//!     capabilities: vec!["scrape".to_string()],
//!     ..Default::default()
//! };
//! let mut client = AuthenticatedSignalingClient::new(config);
//! client.connect().await?;
//! ```

mod signaling;
mod peer;
mod channel;
pub mod auth;
pub mod credentials;
pub mod grpc_handler;

pub use signaling::{
    SignalingClient, SignalingMessage, AuthenticatedSignalingClient,
    SignalingConfig, SignalingEventHandler, AuthOKPayload, AuthErrorPayload,
    AppRegisteredPayload, WSMessage, msg_types, ReconnectConfig,
};
pub use credentials::{P2PCredentials, CredentialsError};
pub use auth::{AuthError, SetupConfig, OAuthSetup};
pub use peer::{P2PPeer, PeerConfig, PeerEvent, TurnServer, ConnectionState, PeerRecreator};
pub use channel::{DataChannel, ChannelMessage};

use thiserror::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Errors that can occur during P2P communication
#[derive(Error, Debug)]
pub enum P2PError {
    #[error("Signaling error: {0}")]
    Signaling(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("WebRTC error: {0}")]
    WebRTC(String),

    #[error("Timeout waiting for peer")]
    Timeout,

    #[error("Peer not found: {0}")]
    PeerNotFound(String),
}

/// Configuration for P2P networking
#[derive(Clone, Debug)]
pub struct P2PConfig {
    /// Signaling server URL for peer discovery
    pub signaling_url: String,

    /// STUN server URLs for NAT traversal
    pub stun_servers: Vec<String>,

    /// TURN server URLs for relay (if STUN fails)
    pub turn_servers: Vec<TurnServer>,

    /// Local peer ID (auto-generated if not specified)
    pub peer_id: Option<String>,

    /// Connection timeout in seconds
    pub connection_timeout_secs: u64,
}

// TurnServer is re-exported from peer module

impl Default for P2PConfig {
    fn default() -> Self {
        Self {
            signaling_url: String::new(),
            stun_servers: vec![
                "stun:stun.l.google.com:19302".to_string(),
                "stun:stun1.l.google.com:19302".to_string(),
            ],
            turn_servers: vec![],
            peer_id: None,
            connection_timeout_secs: 30,
        }
    }
}

/// P2P Network Manager
///
/// Manages peer connections and data channels for P2P communication.
pub struct P2PManager {
    config: P2PConfig,
    peers: Arc<RwLock<std::collections::HashMap<String, Arc<P2PPeer>>>>,
    signaling: SignalingClient,
    local_peer_id: String,
}

impl P2PManager {
    /// Create a new P2P manager with the given configuration
    pub fn new(config: P2PConfig) -> Self {
        let local_peer_id = config.peer_id.clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let signaling = SignalingClient::new(config.signaling_url.clone());

        Self {
            config,
            peers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            signaling,
            local_peer_id,
        }
    }

    /// Get the local peer ID
    pub fn local_peer_id(&self) -> &str {
        &self.local_peer_id
    }

    /// Connect to the signaling server
    pub async fn connect(&mut self) -> Result<(), P2PError> {
        self.signaling.connect(&self.local_peer_id).await
    }

    /// Disconnect from the signaling server
    pub async fn disconnect(&mut self) -> Result<(), P2PError> {
        self.signaling.disconnect().await
    }

    /// Create a peer config from the manager config
    fn create_peer_config(&self) -> PeerConfig {
        PeerConfig {
            stun_servers: self.config.stun_servers.clone(),
            turn_servers: self.config.turn_servers.clone(),
        }
    }

    /// Connect to a remote peer by ID
    pub async fn connect_to_peer(&self, peer_id: &str) -> Result<Arc<P2PPeer>, P2PError> {
        let peer_config = self.create_peer_config();

        let peer = P2PPeer::new(peer_id.to_string(), peer_config).await?;
        peer.setup_handlers().await?;

        // Create offer and send via signaling
        let offer = peer.create_offer().await?;
        self.signaling.send(SignalingMessage::Offer {
            from: self.local_peer_id.clone(),
            to: peer_id.to_string(),
            sdp: offer,
        }).await?;

        // Wait for answer
        let answer = self.wait_for_answer(peer_id).await?;
        peer.set_remote_answer(&answer).await?;

        // Store peer
        let peer = Arc::new(peer);
        self.peers.write().await.insert(peer_id.to_string(), peer.clone());

        Ok(peer)
    }

    /// Wait for an answer from a specific peer
    async fn wait_for_answer(&self, peer_id: &str) -> Result<String, P2PError> {
        let timeout = std::time::Duration::from_secs(self.config.connection_timeout_secs);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(P2PError::Timeout);
            }

            if let Some(msg) = self.signaling.receive().await? {
                if let SignalingMessage::Answer { from, sdp, .. } = msg {
                    if from == peer_id {
                        return Ok(sdp);
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Handle an incoming connection offer
    pub async fn handle_offer(&self, from: &str, sdp: String) -> Result<Arc<P2PPeer>, P2PError> {
        let peer_config = self.create_peer_config();

        let peer = P2PPeer::new(from.to_string(), peer_config).await?;
        peer.setup_handlers().await?;
        peer.setup_data_channel_handler().await?;

        // Create answer
        let answer = peer.create_answer(&sdp).await?;
        self.signaling.send(SignalingMessage::Answer {
            from: self.local_peer_id.clone(),
            to: from.to_string(),
            sdp: answer,
        }).await?;

        // Store peer
        let peer = Arc::new(peer);
        self.peers.write().await.insert(from.to_string(), peer.clone());

        Ok(peer)
    }

    /// Get a connected peer by ID
    pub async fn get_peer(&self, peer_id: &str) -> Option<Arc<P2PPeer>> {
        self.peers.read().await.get(peer_id).cloned()
    }

    /// Send data to a specific peer
    pub async fn send_to_peer(&self, peer_id: &str, data: &[u8]) -> Result<(), P2PError> {
        let peers = self.peers.read().await;
        let peer = peers.get(peer_id)
            .ok_or_else(|| P2PError::PeerNotFound(peer_id.to_string()))?;

        peer.send(data).await
    }

    /// Broadcast data to all connected peers
    pub async fn broadcast(&self, data: &[u8]) -> Result<(), P2PError> {
        let peers = self.peers.read().await;

        for peer in peers.values() {
            if let Err(e) = peer.send(data).await {
                tracing::warn!("Failed to send to peer {}: {:?}", peer.remote_id(), e);
            }
        }

        Ok(())
    }

    /// Get list of connected peer IDs
    pub async fn connected_peers(&self) -> Vec<String> {
        self.peers.read().await.keys().cloned().collect()
    }
}
