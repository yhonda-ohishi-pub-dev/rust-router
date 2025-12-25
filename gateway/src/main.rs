//! Gateway main entry point
//!
//! This is the gRPC gateway that receives external requests
//! and routes them to internal services via InProcess calls.

use std::sync::Arc;

use tokio::sync::RwLock;
use tonic::transport::Server;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use gateway_lib::{
    grpc::gateway_server::proto::{
        etc_scraper_server::EtcScraperServer,
        gateway_service_server::GatewayServiceServer,
    },
    grpc::gateway_service::GatewayServiceImpl,
    p2p::{self, P2PCredentials, SetupConfig},
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

    // Start gRPC server with optional shutdown signal
    let server = Server::builder()
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
                // P2P OAuth setup
                let runtime = tokio::runtime::Runtime::new()?;
                runtime.block_on(run_p2p_setup(None, None))?;
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
    println!();
    println!("Usage:");
    println!("  gateway                  Run as Windows service");
    println!("  gateway run              Run as console application");
    println!("  gateway install          Install as Windows service");
    println!("  gateway uninstall        Uninstall Windows service");
    println!();
    println!("P2P Options:");
    println!("  --p2p-setup              Run OAuth setup for P2P authentication");
    println!("  --p2p-creds <path>       Specify credentials file path");
    println!("  --p2p-apikey <key>       Use specified API key directly");
    println!("  --p2p-auth-url <url>     Auth server URL for OAuth setup");
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
    let mut creds_path = None;
    let mut api_key = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--p2p-auth-url" if i + 1 < args.len() => {
                auth_url = Some(args[i + 1].clone());
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
                let auth_url = auth_url.clone();
                let creds_path = creds_path.clone();
                return Some(Box::pin(async move {
                    run_p2p_setup(auth_url.as_deref(), creds_path.as_deref()).await
                }));
            }
            _ => {
                i += 1;
            }
        }
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
        app_name: "Gateway".to_string(),
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
