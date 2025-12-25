//! Signaling client for WebRTC peer discovery and connection setup
//!
//! Implements WebSocket-based signaling with API key authentication,
//! compatible with cf-wbrtc-auth signaling server.

use super::P2PError;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

/// WebSocket message structure
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WSMessage {
    #[serde(rename = "type")]
    pub msg_type: String,

    pub payload: serde_json::Value,

    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Messages exchanged via the signaling server
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SignalingMessage {
    /// Register with the signaling server
    Register { peer_id: String },

    /// Connection offer (SDP)
    Offer { from: String, to: String, sdp: String },

    /// Connection answer (SDP)
    Answer { from: String, to: String, sdp: String },

    /// ICE candidate for NAT traversal
    IceCandidate {
        from: String,
        to: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },

    /// Peer disconnected
    Disconnect { peer_id: String },

    /// Error from signaling server
    Error { message: String },
}

/// Authentication payload for auth message
#[derive(Debug, Serialize)]
struct AuthPayload {
    #[serde(rename = "apiKey")]
    api_key: String,
}

/// Response from successful auth
#[derive(Debug, Deserialize)]
pub struct AuthOKPayload {
    #[serde(rename = "userId")]
    pub user_id: String,

    #[serde(rename = "type")]
    pub user_type: String,
}

/// Response from failed auth
#[derive(Debug, Deserialize)]
pub struct AuthErrorPayload {
    pub error: String,
}

/// App registration payload
#[derive(Debug, Serialize)]
struct AppRegisterPayload {
    name: String,
    capabilities: Vec<String>,
}

/// Response from successful app registration
#[derive(Debug, Deserialize)]
pub struct AppRegisteredPayload {
    #[serde(rename = "appId")]
    pub app_id: String,
}

/// Offer payload from signaling server
#[derive(Debug, Deserialize)]
struct OfferPayload {
    sdp: String,
    #[serde(rename = "targetAppId")]
    #[allow(dead_code)]
    target_app_id: Option<String>,
}

/// Answer payload
#[derive(Debug, Serialize, Deserialize)]
struct AnswerPayload {
    sdp: String,
    #[serde(rename = "appId", skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
}

/// ICE payload
#[derive(Debug, Serialize, Deserialize)]
struct ICEPayload {
    candidate: serde_json::Value,
    #[serde(rename = "targetAppId", skip_serializing_if = "Option::is_none")]
    target_app_id: Option<String>,
    #[serde(rename = "appId", skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
}

/// Error payload
#[derive(Debug, Deserialize)]
struct ErrorPayload {
    message: String,
}

/// Message types
pub mod msg_types {
    pub const AUTH: &str = "auth";
    pub const AUTH_OK: &str = "auth_ok";
    pub const AUTH_ERROR: &str = "auth_error";
    pub const APP_REGISTER: &str = "app_register";
    pub const APP_REGISTERED: &str = "app_registered";
    pub const APP_STATUS: &str = "app_status";
    pub const GET_APPS: &str = "get_apps";
    pub const APPS_LIST: &str = "apps_list";
    pub const OFFER: &str = "offer";
    pub const ANSWER: &str = "answer";
    pub const ICE: &str = "ice";
    pub const ERROR: &str = "error";
}

/// Event handler trait for signaling events
#[async_trait::async_trait]
pub trait SignalingEventHandler: Send + Sync {
    async fn on_authenticated(&self, payload: AuthOKPayload);
    async fn on_auth_error(&self, payload: AuthErrorPayload);
    async fn on_app_registered(&self, payload: AppRegisteredPayload);
    async fn on_offer(&self, sdp: String, request_id: Option<String>);
    async fn on_answer(&self, sdp: String, app_id: Option<String>);
    async fn on_ice(&self, candidate: serde_json::Value);
    async fn on_error(&self, message: String);
    async fn on_connected(&self);
    async fn on_disconnected(&self);
}

/// Configuration for SignalingClient
#[derive(Clone, Debug)]
pub struct SignalingConfig {
    /// WebSocket URL (e.g., wss://example.com/ws/app)
    pub server_url: String,

    /// API key for authentication
    pub api_key: String,

    /// Application name
    pub app_name: String,

    /// App capabilities (e.g., ["print", "scrape"])
    pub capabilities: Vec<String>,

    /// Ping interval (default: 30s)
    pub ping_interval: Duration,
}

impl Default for SignalingConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            api_key: String::new(),
            app_name: "Gateway".to_string(),
            capabilities: vec![],
            ping_interval: Duration::from_secs(30),
        }
    }
}

/// State of the signaling client
struct ClientState {
    is_connected: bool,
    is_authenticated: bool,
    app_id: String,
}

/// Authenticated signaling client for P2P communication
pub struct AuthenticatedSignalingClient {
    config: SignalingConfig,
    state: Arc<RwLock<ClientState>>,
    send_tx: Option<mpsc::Sender<Message>>,
    event_handler: Option<Arc<dyn SignalingEventHandler>>,
}

impl AuthenticatedSignalingClient {
    /// Create a new authenticated signaling client
    pub fn new(config: SignalingConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(ClientState {
                is_connected: false,
                is_authenticated: false,
                app_id: String::new(),
            })),
            send_tx: None,
            event_handler: None,
        }
    }

    /// Set event handler
    pub fn set_event_handler(&mut self, handler: Arc<dyn SignalingEventHandler>) {
        self.event_handler = Some(handler);
    }

    /// Connect to the signaling server with authentication
    pub async fn connect(&mut self) -> Result<(), P2PError> {
        if self.config.server_url.is_empty() {
            return Err(P2PError::Signaling("Signaling URL not configured".to_string()));
        }

        // Build URL with API key
        let mut url = Url::parse(&self.config.server_url)
            .map_err(|e| P2PError::Signaling(format!("Invalid URL: {}", e)))?;

        url.query_pairs_mut()
            .append_pair("apiKey", &self.config.api_key);

        tracing::debug!("Connecting to signaling server: {}", self.config.server_url);

        // Connect WebSocket
        let (ws_stream, _) = connect_async(url.as_str())
            .await
            .map_err(|e| P2PError::Signaling(format!("WebSocket connection failed: {}", e)))?;

        let (mut write, mut read) = ws_stream.split();

        // Set connected state
        {
            let mut state = self.state.write().await;
            state.is_connected = true;
        }

        // Notify handler
        if let Some(ref handler) = self.event_handler {
            handler.on_connected().await;
        }

        // Create send channel
        let (send_tx, mut send_rx) = mpsc::channel::<Message>(100);
        self.send_tx = Some(send_tx);

        // Send auth message
        self.send_auth().await?;

        // Clone for background tasks
        let state = Arc::clone(&self.state);
        let event_handler = self.event_handler.clone();
        let config = self.config.clone();

        // Spawn write task
        let write_state = Arc::clone(&state);
        tokio::spawn(async move {
            while let Some(msg) = send_rx.recv().await {
                if write.send(msg).await.is_err() {
                    let mut s = write_state.write().await;
                    s.is_connected = false;
                    break;
                }
            }
        });

        // Spawn read task
        tokio::spawn(async move {
            while let Some(result) = read.next().await {
                match result {
                    Ok(Message::Text(text)) => {
                        Self::handle_message(&state, &event_handler, &config, &text).await;
                    }
                    Ok(Message::Close(_)) => {
                        let mut s = state.write().await;
                        s.is_connected = false;
                        s.is_authenticated = false;
                        if let Some(ref handler) = event_handler {
                            handler.on_disconnected().await;
                        }
                        break;
                    }
                    Err(e) => {
                        tracing::error!("WebSocket error: {}", e);
                        if let Some(ref handler) = event_handler {
                            handler.on_error(format!("WebSocket error: {}", e)).await;
                        }
                        break;
                    }
                    _ => {}
                }
            }

            let mut s = state.write().await;
            s.is_connected = false;
            s.is_authenticated = false;
        });

        Ok(())
    }

    /// Handle incoming message
    async fn handle_message(
        state: &Arc<RwLock<ClientState>>,
        event_handler: &Option<Arc<dyn SignalingEventHandler>>,
        config: &SignalingConfig,
        text: &str,
    ) {
        let msg: WSMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Failed to parse message: {}", e);
                return;
            }
        };

        match msg.msg_type.as_str() {
            msg_types::AUTH_OK => {
                if let Ok(payload) = serde_json::from_value::<AuthOKPayload>(msg.payload) {
                    {
                        let mut s = state.write().await;
                        s.is_authenticated = true;
                    }
                    if let Some(ref handler) = event_handler {
                        handler.on_authenticated(payload).await;
                    }
                    // Auto-register app after auth - send via event handler
                    tracing::info!(
                        "Authenticated, registering app: {}",
                        config.app_name
                    );
                }
            }
            msg_types::AUTH_ERROR => {
                if let Ok(payload) = serde_json::from_value::<AuthErrorPayload>(msg.payload) {
                    if let Some(ref handler) = event_handler {
                        handler.on_auth_error(payload).await;
                    }
                }
            }
            msg_types::APP_REGISTERED => {
                if let Ok(payload) = serde_json::from_value::<AppRegisteredPayload>(msg.payload) {
                    {
                        let mut s = state.write().await;
                        s.app_id = payload.app_id.clone();
                    }
                    if let Some(ref handler) = event_handler {
                        handler.on_app_registered(payload).await;
                    }
                }
            }
            msg_types::OFFER => {
                if let Ok(payload) = serde_json::from_value::<OfferPayload>(msg.payload) {
                    if let Some(ref handler) = event_handler {
                        handler.on_offer(payload.sdp, msg.request_id).await;
                    }
                }
            }
            msg_types::ANSWER => {
                if let Ok(payload) = serde_json::from_value::<AnswerPayload>(msg.payload) {
                    if let Some(ref handler) = event_handler {
                        handler.on_answer(payload.sdp, payload.app_id).await;
                    }
                }
            }
            msg_types::ICE => {
                if let Ok(payload) = serde_json::from_value::<ICEPayload>(msg.payload) {
                    if let Some(ref handler) = event_handler {
                        handler.on_ice(payload.candidate).await;
                    }
                }
            }
            msg_types::ERROR => {
                if let Ok(payload) = serde_json::from_value::<ErrorPayload>(msg.payload) {
                    if let Some(ref handler) = event_handler {
                        handler.on_error(payload.message).await;
                    }
                }
            }
            _ => {
                tracing::debug!("Unknown message type: {}", msg.msg_type);
            }
        }
    }

    /// Send auth message
    async fn send_auth(&self) -> Result<(), P2PError> {
        let payload = AuthPayload {
            api_key: self.config.api_key.clone(),
        };
        self.send_message(msg_types::AUTH, serde_json::to_value(payload).unwrap(), None)
            .await
    }

    /// Register app with name and capabilities
    pub async fn register_app(&self) -> Result<(), P2PError> {
        let payload = AppRegisterPayload {
            name: self.config.app_name.clone(),
            capabilities: self.config.capabilities.clone(),
        };
        self.send_message(
            msg_types::APP_REGISTER,
            serde_json::to_value(payload).unwrap(),
            None,
        )
        .await
    }

    /// Send WebRTC answer SDP
    pub async fn send_answer(&self, sdp: &str, request_id: Option<&str>) -> Result<(), P2PError> {
        let payload = AnswerPayload {
            sdp: sdp.to_string(),
            app_id: None,
        };
        self.send_message(
            msg_types::ANSWER,
            serde_json::to_value(payload).unwrap(),
            request_id.map(|s| s.to_string()),
        )
        .await
    }

    /// Send ICE candidate
    pub async fn send_ice(&self, candidate: serde_json::Value) -> Result<(), P2PError> {
        let payload = ICEPayload {
            candidate,
            target_app_id: None,
            app_id: None,
        };
        self.send_message(msg_types::ICE, serde_json::to_value(payload).unwrap(), None)
            .await
    }

    /// Send a message to the signaling server
    async fn send_message(
        &self,
        msg_type: &str,
        payload: serde_json::Value,
        request_id: Option<String>,
    ) -> Result<(), P2PError> {
        let msg = WSMessage {
            msg_type: msg_type.to_string(),
            payload,
            request_id,
        };

        let json = serde_json::to_string(&msg)
            .map_err(|e| P2PError::Signaling(format!("Failed to serialize message: {}", e)))?;

        if let Some(ref tx) = self.send_tx {
            tx.send(Message::Text(json.into()))
                .await
                .map_err(|e| P2PError::Signaling(format!("Failed to send message: {}", e)))?;
        } else {
            return Err(P2PError::Signaling("Not connected".to_string()));
        }

        Ok(())
    }

    /// Close the connection
    pub async fn close(&mut self) -> Result<(), P2PError> {
        let mut state = self.state.write().await;
        state.is_connected = false;
        state.is_authenticated = false;
        self.send_tx = None;
        Ok(())
    }

    /// Check if connected and authenticated
    pub async fn is_connected(&self) -> bool {
        let state = self.state.read().await;
        state.is_connected && state.is_authenticated
    }

    /// Get the registered app ID
    pub async fn get_app_id(&self) -> String {
        self.state.read().await.app_id.clone()
    }
}

// Keep the legacy SignalingClient for backwards compatibility
/// Client for communicating with a signaling server (legacy, non-authenticated)
pub struct SignalingClient {
    url: String,
    connected: Arc<RwLock<bool>>,
    send_tx: Option<mpsc::Sender<SignalingMessage>>,
    recv_rx: Arc<RwLock<Option<mpsc::Receiver<SignalingMessage>>>>,
}

impl SignalingClient {
    /// Create a new signaling client
    pub fn new(url: String) -> Self {
        Self {
            url,
            connected: Arc::new(RwLock::new(false)),
            send_tx: None,
            recv_rx: Arc::new(RwLock::new(None)),
        }
    }

    /// Connect to the signaling server
    pub async fn connect(&mut self, peer_id: &str) -> Result<(), P2PError> {
        if self.url.is_empty() {
            return Err(P2PError::Signaling("Signaling URL not configured".to_string()));
        }

        let (send_tx, mut send_rx) = mpsc::channel::<SignalingMessage>(100);
        let (recv_tx, recv_rx) = mpsc::channel::<SignalingMessage>(100);

        self.send_tx = Some(send_tx);
        *self.recv_rx.write().await = Some(recv_rx);
        *self.connected.write().await = true;

        let url = self.url.clone();
        let peer_id = peer_id.to_string();

        tokio::spawn(async move {
            tracing::info!("Signaling client connected to {}", url);

            let _ = recv_tx
                .send(SignalingMessage::Register {
                    peer_id: peer_id.clone(),
                })
                .await;

            while let Some(msg) = send_rx.recv().await {
                tracing::debug!("Signaling: sending {:?}", msg);
            }

            tracing::info!("Signaling client disconnected");
        });

        Ok(())
    }

    /// Disconnect from the signaling server
    pub async fn disconnect(&mut self) -> Result<(), P2PError> {
        *self.connected.write().await = false;
        self.send_tx = None;
        *self.recv_rx.write().await = None;
        Ok(())
    }

    /// Check if connected to the signaling server
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// Send a signaling message
    pub async fn send(&self, message: SignalingMessage) -> Result<(), P2PError> {
        if !self.is_connected().await {
            return Err(P2PError::Signaling(
                "Not connected to signaling server".to_string(),
            ));
        }

        if let Some(ref tx) = self.send_tx {
            tx.send(message)
                .await
                .map_err(|e| P2PError::Signaling(format!("Failed to send message: {}", e)))?;
        }

        Ok(())
    }

    /// Receive a signaling message (non-blocking)
    pub async fn receive(&self) -> Result<Option<SignalingMessage>, P2PError> {
        let mut recv_rx = self.recv_rx.write().await;

        if let Some(ref mut rx) = *recv_rx {
            match rx.try_recv() {
                Ok(msg) => Ok(Some(msg)),
                Err(mpsc::error::TryRecvError::Empty) => Ok(None),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    Err(P2PError::Signaling("Channel disconnected".to_string()))
                }
            }
        } else {
            Ok(None)
        }
    }
}
