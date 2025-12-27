//! Gateway main entry point
//!
//! This is the gRPC gateway that receives external requests
//! and routes them to internal services via InProcess calls.

use std::sync::Arc;

use tokio::sync::RwLock;
use tonic::transport::Server;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use gateway_lib::{
    grpc::gateway_server::gateway_service_server::GatewayServiceServer,
    grpc::scraper_server::etc_scraper_server::EtcScraperServer,
    grpc::gateway_service::GatewayServiceImpl,
    p2p::{self, grpc_handler::TonicServiceBridge, P2PCredentials, SetupConfig},
    updater::{AutoUpdater, UpdateConfig, UpdateChannel, format_update_info},
    EtcScraperService, GatewayConfig, JobQueue,
};

#[cfg(windows)]
mod windows_service_impl {
    use std::{ffi::OsString, time::Duration};
    use windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
    };

    const SERVICE_NAME: &str = "GatewayService";
    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    define_windows_service!(ffi_service_main, service_main);

    pub fn run_as_service() -> Result<(), windows_service::Error> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }

    fn service_main(_arguments: Vec<OsString>) {
        if let Err(e) = run_service() {
            tracing::error!("Service error: {:?}", e);
        }
    }

    fn run_service() -> Result<(), Box<dyn std::error::Error>> {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let shutdown_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(shutdown_tx)));

        let shutdown_tx_clone = shutdown_tx.clone();
        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    if let Some(tx) = shutdown_tx_clone.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            super::run_server(Some(shutdown_rx)).await
        })?;

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        Ok(())
    }
}

async fn run_server(
    shutdown_rx: Option<tokio::sync::oneshot::Receiver<()>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "gateway=info".into());

    let is_service = shutdown_rx.is_some();

    #[cfg(windows)]
    if is_service {
        // Windows Service mode: output to both Event Log and console
        let eventlog = tracing_layer_win_eventlog::EventLogLayer::new("GatewayService".to_string());
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .with(eventlog)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    #[cfg(not(windows))]
    {
        let _ = is_service; // suppress unused warning
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    // Load configuration
    let config = GatewayConfig::from_env();
    tracing::info!("Starting Gateway v{}", config.version);
    tracing::info!("gRPC server listening on {}", config.grpc_addr);

    // Create shared job queue
    let job_queue = Arc::new(RwLock::new(JobQueue::new()));

    // Create gRPC services
    let gateway_service = GatewayServiceImpl::new();
    let scraper_service = EtcScraperService::new(config.clone(), job_queue.clone());

    // Parse address
    let addr = config.grpc_addr.parse()?;

    // Create reflection service
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("Failed to create reflection service");

    // Start gRPC server with optional shutdown signal
    let server = Server::builder()
        .add_service(reflection_service)
        .add_service(GatewayServiceServer::new(gateway_service))
        .add_service(EtcScraperServer::new(scraper_service));

    match shutdown_rx {
        Some(rx) => {
            server
                .serve_with_shutdown(addr, async {
                    let _ = rx.await;
                    tracing::info!("Shutdown signal received");
                })
                .await?;
        }
        None => {
            server.serve(addr).await?;
        }
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Check for command line arguments
    if args.len() > 1 {
        match args[1].as_str() {
            "install" => {
                #[cfg(windows)]
                {
                    install_service()?;
                    println!("Service installed successfully");
                    return Ok(());
                }
                #[cfg(not(windows))]
                {
                    eprintln!("Service installation is only supported on Windows");
                    return Ok(());
                }
            }
            "uninstall" => {
                #[cfg(windows)]
                {
                    uninstall_service()?;
                    println!("Service uninstalled successfully");
                    return Ok(());
                }
                #[cfg(not(windows))]
                {
                    eprintln!("Service uninstallation is only supported on Windows");
                    return Ok(());
                }
            }
            "run" => {
                // Run as console application
                let runtime = tokio::runtime::Runtime::new()?;
                runtime.block_on(run_server(None))?;
                return Ok(());
            }
            "--p2p-setup" => {
                // P2P OAuth setup - fall through to parse_p2p_args to collect all options
                if let Some(result) = parse_p2p_args(&args) {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(result)?;
                    return Ok(());
                }
            }
            "--check-update" => {
                // Check for updates
                let runtime = tokio::runtime::Runtime::new()?;
                let channel = find_update_channel(&args);
                runtime.block_on(check_for_update(channel))?;
                return Ok(());
            }
            "--update" => {
                // Perform update (exe)
                let runtime = tokio::runtime::Runtime::new()?;
                let channel = find_update_channel(&args);
                runtime.block_on(perform_update(channel, false))?;
                return Ok(());
            }
            "--update-msi" => {
                // Perform update using MSI installer
                let runtime = tokio::runtime::Runtime::new()?;
                let channel = find_update_channel(&args);
                runtime.block_on(perform_update(channel, true))?;
                return Ok(());
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {
                // Check for --p2p-* options
                if let Some(result) = parse_p2p_args(&args) {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(result)?;
                    return Ok(());
                }
            }
        }
    }

    // Default: try to run as Windows service
    #[cfg(windows)]
    {
        match windows_service_impl::run_as_service() {
            Ok(_) => Ok(()),
            Err(e) => {
                // If we can't start as a service (e.g., running from console),
                // run as a regular console app
                eprintln!("Failed to start as service: {:?}", e);
                eprintln!("Running as console application instead...");
                eprintln!("Use 'gateway run' to run as console app, or 'gateway install' to install as service");
                let runtime = tokio::runtime::Runtime::new()?;
                runtime.block_on(run_server(None))
            }
        }
    }

    #[cfg(not(windows))]
    {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(run_server(None))
    }
}

#[cfg(windows)]
fn install_service() -> Result<(), Box<dyn std::error::Error>> {
    use std::ffi::OsString;
    use windows_service::{
        service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let service_binary_path = std::env::current_exe()?;

    let service_info = ServiceInfo {
        name: OsString::from("GatewayService"),
        display_name: OsString::from("API Gateway Service"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_binary_path,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let _service = manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;

    Ok(())
}

#[cfg(windows)]
fn uninstall_service() -> Result<(), Box<dyn std::error::Error>> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )?;

    let service = manager.open_service(
        "GatewayService",
        ServiceAccess::DELETE,
    )?;

    service.delete()?;

    Ok(())
}

fn print_help() {
    println!("Gateway Service - API Gateway for gRPC requests");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage:");
    println!("  gateway                  Run as Windows service");
    println!("  gateway run              Run as console application");
    println!("  gateway install          Install as Windows service");
    println!("  gateway uninstall        Uninstall Windows service");
    println!();
    println!("Update Options:");
    println!("  --check-update           Check for available updates");
    println!("  --update                 Download and install the latest update (exe)");
    println!("  --update-msi             Download and install the latest update (MSI installer)");
    println!("  --update-channel <ch>    Update channel: stable (default) or beta");
    println!();
    println!("P2P Options:");
    println!("  --p2p-setup              Run OAuth setup for P2P authentication");
    println!("  --p2p-run                Connect to P2P signaling server");
    println!("  --p2p-creds <path>       Specify credentials file path");
    println!("  --p2p-apikey <key>       Use specified API key directly");
    println!("  --p2p-auth-url <url>     Auth server URL for OAuth setup");
    println!("  --p2p-signaling-url <url> Signaling server WebSocket URL");
    println!();
    println!("Environment Variables:");
    println!("  GATEWAY_GRPC_ADDR        gRPC listen address (default: [::1]:50051)");
    println!("  P2P_AUTH_URL             Auth server URL for P2P OAuth");
    println!("  P2P_SIGNALING_URL        WebSocket signaling server URL");
    println!("  GITHUB_OWNER             GitHub repository owner for updates");
    println!("  GITHUB_REPO              GitHub repository name for updates");
}

/// Parse P2P-related command line arguments
fn parse_p2p_args(
    args: &[String],
) -> Option<std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Box<dyn std::error::Error>>> + Send>>> {
    let mut auth_url = std::env::var("P2P_AUTH_URL").ok();
    let mut signaling_url = std::env::var("P2P_SIGNALING_URL").ok();
    let mut creds_path = None;
    let mut api_key = None;
    let mut has_setup = false;
    let mut has_run = false;

    // First pass: collect all arguments
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--p2p-auth-url" if i + 1 < args.len() => {
                auth_url = Some(args[i + 1].clone());
                i += 2;
            }
            "--p2p-signaling-url" if i + 1 < args.len() => {
                signaling_url = Some(args[i + 1].clone());
                i += 2;
            }
            "--p2p-creds" if i + 1 < args.len() => {
                creds_path = Some(args[i + 1].clone());
                i += 2;
            }
            "--p2p-apikey" if i + 1 < args.len() => {
                api_key = Some(args[i + 1].clone());
                i += 2;
            }
            "--p2p-setup" => {
                has_setup = true;
                i += 1;
            }
            "--p2p-run" => {
                has_run = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    // Second pass: execute based on collected arguments
    if has_setup {
        return Some(Box::pin(async move {
            run_p2p_setup(auth_url.as_deref(), creds_path.as_deref()).await
        }));
    }

    if has_run {
        return Some(Box::pin(async move {
            run_p2p_client(signaling_url, creds_path).await
        }));
    }

    // If we have an API key specified, save it
    if let Some(key) = api_key {
        let creds_path = creds_path.clone();
        return Some(Box::pin(async move {
            save_api_key(&key, creds_path.as_deref()).await
        }));
    }

    None
}

/// Run P2P OAuth setup
async fn run_p2p_setup(
    auth_url: Option<&str>,
    creds_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for setup
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "gateway=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let auth_url = auth_url
        .map(|s| s.to_string())
        .or_else(|| std::env::var("P2P_AUTH_URL").ok())
        .ok_or("P2P auth server URL not specified. Use --p2p-auth-url or set P2P_AUTH_URL")?;

    println!("Starting P2P OAuth setup...");
    println!("Auth server: {}", auth_url);

    let config = SetupConfig {
        auth_server_url: auth_url,
        app_name: "gateway-pc".to_string(),
        auto_open_browser: true,
        ..Default::default()
    };

    let credentials = p2p::auth::load_or_setup(creds_path, config).await
        .map_err(|e| format!("OAuth setup failed: {}", e))?;

    println!();
    println!("Setup completed successfully!");
    println!("API Key: {}...", &credentials.api_key[..credentials.api_key.len().min(20)]);
    if !credentials.app_id.is_empty() {
        println!("App ID: {}", credentials.app_id);
    }

    let path = creds_path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(P2PCredentials::default_path);
    println!("Credentials saved to: {}", path.display());

    Ok(())
}

/// Save API key directly to credentials file
async fn save_api_key(
    api_key: &str,
    creds_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let creds = P2PCredentials::new(api_key.to_string());
    let path = creds_path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(P2PCredentials::default_path);

    creds.save(&path)?;
    println!("API key saved to: {}", path.display());

    Ok(())
}

/// Run P2P client and connect to signaling server
async fn run_p2p_client(
    signaling_url: Option<String>,
    creds_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "gateway=debug,webrtc=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load credentials
    let path = creds_path
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(P2PCredentials::default_path);

    let creds = P2PCredentials::load(&path)
        .map_err(|e| format!("Failed to load credentials from {}: {}", path.display(), e))?;

    println!("Loaded credentials from: {}", path.display());
    println!("API Key: {}...", &creds.api_key[..creds.api_key.len().min(20)]);

    // Determine signaling URL
    let signaling_url = signaling_url
        .or_else(|| std::env::var("P2P_SIGNALING_URL").ok())
        .unwrap_or_else(|| "wss://cf-wbrtc-auth.m-tama-ramu.workers.dev/ws/app".to_string());

    println!("Connecting to signaling server: {}", signaling_url);

    // Shared state for P2P peer management with multi-peer support
    struct P2PState {
        signaling_client: Option<Arc<RwLock<p2p::AuthenticatedSignalingClient>>>,
        /// Map of peer_id -> peer connection
        peers: HashMap<String, Arc<p2p::P2PPeer>>,
        /// Counter for generating unique peer IDs
        peer_counter: u64,
    }

    impl P2PState {
        fn new() -> Self {
            Self {
                signaling_client: None,
                peers: HashMap::new(),
                peer_counter: 0,
            }
        }

        /// Generate a unique peer ID
        fn next_peer_id(&mut self) -> String {
            self.peer_counter += 1;
            format!("peer-{}", self.peer_counter)
        }

        /// Remove a peer from the map and return it for cleanup
        fn remove_peer(&mut self, peer_id: &str) -> Option<Arc<p2p::P2PPeer>> {
            self.peers.remove(peer_id)
        }

        /// Get current peer count
        fn peer_count(&self) -> usize {
            self.peers.len()
        }
    }

    let state = Arc::new(RwLock::new(P2PState::new()));

    // Create gRPC service and bridge for P2P requests
    let config = GatewayConfig::from_env();
    let job_queue = Arc::new(RwLock::new(JobQueue::new()));
    let scraper_service = EtcScraperService::new(config, job_queue);
    let grpc_server = EtcScraperServer::new(scraper_service);
    let grpc_bridge = Arc::new(TonicServiceBridge::new(grpc_server));

    // Type alias for the gRPC bridge with EtcScraperServer
    type ScraperBridge = TonicServiceBridge<EtcScraperServer<EtcScraperService>>;

    // Create event handler with state access
    struct P2PEventHandler {
        state: Arc<RwLock<P2PState>>,
        grpc_bridge: Arc<ScraperBridge>,
    }

    #[async_trait::async_trait]
    impl p2p::SignalingEventHandler for P2PEventHandler {
        async fn on_authenticated(&self, payload: p2p::AuthOKPayload) {
            println!("Authenticated! User ID: {}, Type: {}", payload.user_id, payload.user_type);
        }

        async fn on_auth_error(&self, payload: p2p::AuthErrorPayload) {
            eprintln!("Auth error: {}", payload.error);
        }

        async fn on_app_registered(&self, payload: p2p::AppRegisteredPayload) {
            println!("App registered! App ID: {}", payload.app_id);
            println!("Waiting for WebRTC offers from browsers...");
        }

        async fn on_offer(&self, sdp: String, request_id: Option<String>) {
            // Generate a unique peer ID for this connection
            let peer_id = {
                let mut state = self.state.write().await;
                state.next_peer_id()
            };

            println!("Received WebRTC offer (peer_id: {}, request_id: {:?})", peer_id, request_id);
            tracing::debug!("Offer SDP:\n{}", sdp);

            // Create WebRTC peer and generate answer
            let peer_config = p2p::PeerConfig {
                stun_servers: vec![
                    "stun:stun.l.google.com:19302".to_string(),
                    "stun:stun1.l.google.com:19302".to_string(),
                ],
                turn_servers: vec![],
            };

            match p2p::P2PPeer::new(peer_id.clone(), peer_config).await {
                Ok(peer) => {
                    // Set up handlers
                    if let Err(e) = peer.setup_handlers().await {
                        eprintln!("Failed to setup peer handlers: {:?}", e);
                        return;
                    }

                    if let Err(e) = peer.setup_data_channel_handler().await {
                        eprintln!("Failed to setup data channel handler: {:?}", e);
                        return;
                    }

                    // Subscribe to peer events
                    let mut event_rx = peer.subscribe().await;
                    let peer = Arc::new(peer);

                    // Spawn event handler task with cleanup on disconnect
                    let peer_clone = peer.clone();
                    let grpc_bridge = self.grpc_bridge.clone();
                    let state_clone = self.state.clone();
                    let peer_id_clone = peer_id.clone();
                    tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            match event {
                                p2p::PeerEvent::Connected => {
                                    tracing::info!("WebRTC peer {} connected!", peer_id_clone);
                                    let state = state_clone.read().await;
                                    tracing::info!("Active peers: {}", state.peer_count());
                                }
                                p2p::PeerEvent::Disconnected => {
                                    tracing::info!("WebRTC peer {} disconnected", peer_id_clone);

                                    // Remove peer from state and cleanup
                                    let removed_peer = {
                                        let mut state = state_clone.write().await;
                                        let peer = state.remove_peer(&peer_id_clone);
                                        tracing::info!("Removed peer {} from state. Remaining peers: {}", peer_id_clone, state.peer_count());
                                        peer
                                    };

                                    // Cleanup peer resources
                                    if let Some(peer) = removed_peer {
                                        if let Err(e) = peer.cleanup().await {
                                            tracing::warn!("Failed to cleanup peer {}: {:?}", peer_id_clone, e);
                                        } else {
                                            tracing::debug!("Peer {} cleanup complete", peer_id_clone);
                                        }
                                    }

                                    break;
                                }
                                p2p::PeerEvent::DataReceived(data) => {
                                    tracing::debug!("Received data ({} bytes) from peer {}", data.len(), peer_id_clone);

                                    // Process gRPC request using TonicServiceBridge
                                    let result = p2p::grpc_handler::process_request_with_service(&data, &grpc_bridge).await;

                                    match result {
                                        p2p::grpc_handler::GrpcProcessResult::Unary(response) => {
                                            // Send single unary response
                                            if let Err(e) = peer_clone.send(&response).await {
                                                eprintln!("Failed to send gRPC response to {}: {:?}", peer_id_clone, e);
                                            } else {
                                                tracing::debug!("Sent unary gRPC response ({} bytes) to {}", response.len(), peer_id_clone);
                                            }
                                        }
                                        p2p::grpc_handler::GrpcProcessResult::Streaming(messages) => {
                                            // Send each stream message individually
                                            tracing::info!("Sending {} stream messages to {}", messages.len(), peer_id_clone);
                                            for (i, msg) in messages.iter().enumerate() {
                                                if let Err(e) = peer_clone.send(msg).await {
                                                    eprintln!("Failed to send stream message {}/{} to {}: {:?}", i + 1, messages.len(), peer_id_clone, e);
                                                    break;
                                                } else {
                                                    tracing::debug!("Sent stream message {}/{} ({} bytes) to {}", i + 1, messages.len(), msg.len(), peer_id_clone);
                                                }
                                            }
                                            tracing::info!("Finished sending stream messages to {}", peer_id_clone);
                                        }
                                    }
                                }
                                p2p::PeerEvent::IceCandidate { candidate, sdp_mid, sdp_mline_index } => {
                                    tracing::debug!("Local ICE candidate for {}: {} (mid: {:?}, index: {:?})",
                                        peer_id_clone, candidate, sdp_mid, sdp_mline_index);
                                }
                                p2p::PeerEvent::Error(e) => {
                                    eprintln!("Peer {} error: {}", peer_id_clone, e);
                                }
                            }
                        }
                        tracing::debug!("Event handler task for peer {} exiting", peer_id_clone);
                    });

                    // Create answer SDP
                    match peer.create_answer(&sdp).await {
                        Ok(answer_sdp) => {
                            println!("Created WebRTC answer for peer {}", peer_id);
                            tracing::debug!("Answer SDP:\n{}", answer_sdp);

                            // Send answer via signaling
                            let state = self.state.read().await;
                            if let Some(ref client) = state.signaling_client {
                                let client = client.read().await;
                                if let Err(e) = client.send_answer(&answer_sdp, request_id.as_deref()).await {
                                    eprintln!("Failed to send answer: {:?}", e);
                                } else {
                                    println!("Answer sent successfully for peer {}!", peer_id);

                                    // Wait a moment for ICE gathering
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                                    // Send local ICE candidates
                                    let candidates = peer.get_ice_candidates().await;
                                    for c in candidates {
                                        let candidate_json = serde_json::json!({
                                            "candidate": c.candidate,
                                            "sdpMid": c.sdp_mid,
                                            "sdpMLineIndex": c.sdp_mline_index,
                                        });
                                        if let Err(e) = client.send_ice(candidate_json).await {
                                            tracing::warn!("Failed to send ICE candidate: {:?}", e);
                                        }
                                    }
                                }
                            }

                            // Store peer in state map
                            drop(state);
                            let mut state = self.state.write().await;
                            state.peers.insert(peer_id.clone(), peer);
                            tracing::info!("Peer {} added to state. Total peers: {}", peer_id, state.peer_count());
                        }
                        Err(e) => {
                            eprintln!("Failed to create answer: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to create peer connection: {:?}", e);
                }
            }
        }

        async fn on_answer(&self, sdp: String, app_id: Option<String>) {
            println!("Received answer (app_id: {:?})", app_id);
            tracing::debug!("Answer SDP: {}", &sdp[..sdp.len().min(200)]);

            // Apply answer to existing peer connection (if we were the offerer)
            // For multi-peer, we would need to identify which peer this is for
            // Currently this is mainly for when we are the offerer (not typical in this setup)
            let state = self.state.read().await;
            // Try to find the most recent peer that might be waiting for an answer
            if let Some((_id, peer)) = state.peers.iter().next() {
                if let Err(e) = peer.set_remote_answer(&sdp).await {
                    eprintln!("Failed to set remote answer: {:?}", e);
                } else {
                    println!("Remote answer set successfully");
                }
            }
        }

        async fn on_ice(&self, candidate: serde_json::Value) {
            tracing::debug!("Received remote ICE candidate: {:?}", candidate);

            // Add ICE candidate to all peer connections
            // In a more complete implementation, we'd identify which peer this is for
            let state = self.state.read().await;
            let candidate_str = candidate.get("candidate")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sdp_mid = candidate.get("sdpMid")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let sdp_mline_index = candidate.get("sdpMLineIndex")
                .and_then(|v| v.as_u64())
                .map(|v| v as u16);

            if !candidate_str.is_empty() {
                // Add to all peers (in practice, should be targeted to specific peer)
                for (peer_id, peer) in state.peers.iter() {
                    if let Err(e) = peer.add_ice_candidate(candidate_str, sdp_mid.clone(), sdp_mline_index).await {
                        tracing::warn!("Failed to add ICE candidate to peer {}: {:?}", peer_id, e);
                    } else {
                        tracing::debug!("Added remote ICE candidate to peer {}", peer_id);
                    }
                }
            }
        }

        async fn on_error(&self, message: String) {
            eprintln!("Signaling error: {}", message);
        }

        async fn on_connected(&self) {
            tracing::info!("Connected to signaling server!");
            println!("Connected to signaling server!");
        }

        async fn on_disconnected(&self) {
            tracing::warn!("Disconnected from signaling server");
            println!("Disconnected from signaling server");

            // Cleanup all peers when signaling disconnects
            let peers_to_cleanup: Vec<(String, Arc<p2p::P2PPeer>)> = {
                let mut state = self.state.write().await;
                let peers: Vec<_> = state.peers.drain().collect();
                tracing::info!("Cleaning up {} peers due to signaling disconnect", peers.len());
                peers
            };

            for (peer_id, peer) in peers_to_cleanup {
                tracing::info!("Cleaning up peer {} due to signaling disconnect", peer_id);
                if let Err(e) = peer.cleanup().await {
                    tracing::warn!("Failed to cleanup peer {}: {:?}", peer_id, e);
                }
            }
        }
    }

    // Create signaling client
    let signaling_config = p2p::SignalingConfig {
        server_url: signaling_url,
        api_key: creds.api_key.clone(),
        app_name: "gateway-pc".to_string(),
        capabilities: vec!["scrape".to_string()],
        ..Default::default()
    };

    let mut client = p2p::AuthenticatedSignalingClient::new(signaling_config);
    let handler = Arc::new(P2PEventHandler {
        state: state.clone(),
        grpc_bridge: grpc_bridge.clone(),
    });
    client.set_event_handler(handler);

    // Connect
    client.connect().await
        .map_err(|e| format!("Failed to connect: {:?}", e))?;

    println!("Waiting for authentication...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Store client in state for answer sending
    let client = Arc::new(RwLock::new(client));
    {
        let mut s = state.write().await;
        s.signaling_client = Some(client.clone());
    }

    // Register app after auth
    {
        let client = client.read().await;
        if client.is_connected().await {
            println!("Registering app...");
            client.register_app().await
                .map_err(|e| format!("Failed to register app: {:?}", e))?;
        }
    }

    println!();
    println!("P2P client running. Waiting for WebRTC connections...");
    println!("Press Ctrl+C to exit.");
    println!();

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;

    println!("Shutting down...");
    tracing::info!("Shutdown signal received");

    // Close all peer connections
    {
        let peers_to_close: Vec<(String, Arc<p2p::P2PPeer>)> = {
            let mut state = state.write().await;
            let peers: Vec<_> = state.peers.drain().collect();
            tracing::info!("Closing {} peer connections", peers.len());
            peers
        };

        for (peer_id, peer) in peers_to_close {
            tracing::info!("Closing peer {}", peer_id);
            if let Err(e) = peer.cleanup().await {
                tracing::warn!("Failed to cleanup peer {}: {:?}", peer_id, e);
            }
        }
    }

    // Close signaling client
    {
        let mut client = client.write().await;
        tracing::info!("Closing signaling client");
        client.close().await
            .map_err(|e| format!("Failed to close: {:?}", e))?;
    }

    tracing::info!("Shutdown complete");
    Ok(())
}

/// Find --update-channel argument value
fn find_update_channel(args: &[String]) -> UpdateChannel {
    for i in 0..args.len() {
        if args[i] == "--update-channel" && i + 1 < args.len() {
            return args[i + 1].parse().unwrap_or_default();
        }
    }
    UpdateChannel::default()
}

/// Get update configuration from environment or defaults
fn get_update_config(channel: UpdateChannel) -> UpdateConfig {
    let owner = std::env::var("GITHUB_OWNER")
        .unwrap_or_else(|_| "yhonda-ohishi-pub-dev".to_string());
    let repo = std::env::var("GITHUB_REPO")
        .unwrap_or_else(|_| "rust-router".to_string());

    UpdateConfig::new_github(owner, repo).with_channel(channel)
}

/// Check for available updates
async fn check_for_update(channel: UpdateChannel) -> Result<(), Box<dyn std::error::Error>> {
    println!("Checking for updates (channel: {})...", channel);
    println!("Current version: {}", env!("CARGO_PKG_VERSION"));
    println!();

    let config = get_update_config(channel);
    let updater = AutoUpdater::new(config);

    match updater.check_for_update().await {
        Ok(Some(version)) => {
            println!("Update available!");
            println!();
            println!("{}", format_update_info(&version, env!("CARGO_PKG_VERSION")));
            println!();
            println!("Run 'gateway --update' to install the update.");
        }
        Ok(None) => {
            println!("You are running the latest version.");
        }
        Err(e) => {
            eprintln!("Failed to check for updates: {}", e);
            return Err(e.into());
        }
    }

    wait_for_keypress();
    Ok(())
}

/// Wait for user to press Enter
fn wait_for_keypress() {
    println!();
    println!("Press Enter to exit...");
    let _ = std::io::stdin().read_line(&mut String::new());
}

/// Perform the update
async fn perform_update(channel: UpdateChannel, prefer_msi: bool) -> Result<(), Box<dyn std::error::Error>> {
    let update_type = if prefer_msi { "MSI" } else { "exe" };
    println!("Starting update (channel: {}, type: {})...", channel, update_type);
    println!("Current version: {}", env!("CARGO_PKG_VERSION"));
    println!();

    let config = get_update_config(channel).with_prefer_msi(prefer_msi);
    let updater = AutoUpdater::new(config);

    // First check if update is available
    match updater.check_for_update().await {
        Ok(Some(version)) => {
            println!("Update available: {} -> {}", env!("CARGO_PKG_VERSION"), version.version);
            if let Some(ref notes) = version.release_notes {
                println!();
                println!("Release notes:");
                for line in notes.lines().take(5) {
                    println!("  {}", line);
                }
            }
            println!();
            println!("Downloading...");

            match updater.update_to_version(&version).await {
                Ok(()) => {
                    println!();
                    println!("Update downloaded and staged.");
                    println!("The application will restart to complete the update.");
                    println!();

                    // Exit to allow the update script to replace the executable
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to install update: {}", e);
                    return Err(e.into());
                }
            }
        }
        Ok(None) => {
            println!("You are already running the latest version.");
        }
        Err(e) => {
            eprintln!("Failed to check for updates: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}
