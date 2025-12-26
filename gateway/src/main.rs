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
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gateway=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

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
    println!();
    println!("Usage:");
    println!("  gateway                  Run as Windows service");
    println!("  gateway run              Run as console application");
    println!("  gateway install          Install as Windows service");
    println!("  gateway uninstall        Uninstall Windows service");
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

    // Shared state for P2P peer management
    struct P2PState {
        signaling_client: Option<Arc<RwLock<p2p::AuthenticatedSignalingClient>>>,
        peer: Option<Arc<p2p::P2PPeer>>,
    }

    let state = Arc::new(RwLock::new(P2PState {
        signaling_client: None,
        peer: None,
    }));

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
            println!("Received WebRTC offer (request_id: {:?})", request_id);
            tracing::debug!("Offer SDP:\n{}", sdp);

            // Create WebRTC peer and generate answer
            let peer_config = p2p::PeerConfig {
                stun_servers: vec![
                    "stun:stun.l.google.com:19302".to_string(),
                    "stun:stun1.l.google.com:19302".to_string(),
                ],
                turn_servers: vec![],
            };

            match p2p::P2PPeer::new("browser".to_string(), peer_config).await {
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

                    // Spawn event handler task
                    let peer_clone = peer.clone();
                    let grpc_bridge = self.grpc_bridge.clone();
                    tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            match event {
                                p2p::PeerEvent::Connected => {
                                    println!("WebRTC peer connected!");
                                }
                                p2p::PeerEvent::Disconnected => {
                                    println!("WebRTC peer disconnected");
                                    break;
                                }
                                p2p::PeerEvent::DataReceived(data) => {
                                    tracing::debug!("Received data ({} bytes)", data.len());

                                    // Process gRPC request using TonicServiceBridge
                                    let result = p2p::grpc_handler::process_request_with_service(&data, &grpc_bridge).await;

                                    match result {
                                        p2p::grpc_handler::GrpcProcessResult::Unary(response) => {
                                            // Send single unary response
                                            if let Err(e) = peer_clone.send(&response).await {
                                                eprintln!("Failed to send gRPC response: {:?}", e);
                                            } else {
                                                tracing::debug!("Sent unary gRPC response ({} bytes)", response.len());
                                            }
                                        }
                                        p2p::grpc_handler::GrpcProcessResult::Streaming(messages) => {
                                            // Send each stream message individually
                                            tracing::info!("Sending {} stream messages", messages.len());
                                            for (i, msg) in messages.iter().enumerate() {
                                                if let Err(e) = peer_clone.send(msg).await {
                                                    eprintln!("Failed to send stream message {}/{}: {:?}", i + 1, messages.len(), e);
                                                    break;
                                                } else {
                                                    tracing::debug!("Sent stream message {}/{} ({} bytes)", i + 1, messages.len(), msg.len());
                                                }
                                            }
                                            tracing::info!("Finished sending stream messages");
                                        }
                                    }
                                }
                                p2p::PeerEvent::IceCandidate { candidate, sdp_mid, sdp_mline_index } => {
                                    tracing::debug!("Local ICE candidate: {} (mid: {:?}, index: {:?})",
                                        candidate, sdp_mid, sdp_mline_index);
                                }
                                p2p::PeerEvent::Error(e) => {
                                    eprintln!("Peer error: {}", e);
                                }
                            }
                        }
                    });

                    // Create answer SDP
                    match peer.create_answer(&sdp).await {
                        Ok(answer_sdp) => {
                            println!("Created WebRTC answer");
                            tracing::debug!("Answer SDP:\n{}", answer_sdp);

                            // Send answer via signaling
                            let state = self.state.read().await;
                            if let Some(ref client) = state.signaling_client {
                                let client = client.read().await;
                                if let Err(e) = client.send_answer(&answer_sdp, request_id.as_deref()).await {
                                    eprintln!("Failed to send answer: {:?}", e);
                                } else {
                                    println!("Answer sent successfully!");

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

                            // Store peer in state
                            drop(state);
                            let mut state = self.state.write().await;
                            state.peer = Some(peer);
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
            let state = self.state.read().await;
            if let Some(ref peer) = state.peer {
                if let Err(e) = peer.set_remote_answer(&sdp).await {
                    eprintln!("Failed to set remote answer: {:?}", e);
                } else {
                    println!("Remote answer set successfully");
                }
            }
        }

        async fn on_ice(&self, candidate: serde_json::Value) {
            tracing::debug!("Received remote ICE candidate: {:?}", candidate);

            // Add ICE candidate to peer connection
            let state = self.state.read().await;
            if let Some(ref peer) = state.peer {
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
                    if let Err(e) = peer.add_ice_candidate(candidate_str, sdp_mid, sdp_mline_index).await {
                        tracing::warn!("Failed to add ICE candidate: {:?}", e);
                    } else {
                        tracing::debug!("Added remote ICE candidate");
                    }
                }
            }
        }

        async fn on_error(&self, message: String) {
            eprintln!("Signaling error: {}", message);
        }

        async fn on_connected(&self) {
            println!("Connected to signaling server!");
        }

        async fn on_disconnected(&self) {
            println!("Disconnected from signaling server");
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

    // Close peer connection if exists
    {
        let state = state.read().await;
        if let Some(ref peer) = state.peer {
            let _ = peer.close().await;
        }
    }

    // Close signaling client
    {
        let mut client = client.write().await;
        client.close().await
            .map_err(|e| format!("Failed to close: {:?}", e))?;
    }

    Ok(())
}
