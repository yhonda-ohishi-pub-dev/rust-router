use std::sync::Arc;

use tokio::sync::RwLock;
use tonic::transport::Server;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use router_service::{
    proto::etc_scraper_server::EtcScraperServer, EtcScraperService, JobQueue, RouterConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "router_service=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = RouterConfig::from_env();
    tracing::info!("Starting ETC Scraper Router v{}", config.version);
    tracing::info!("gRPC server listening on {}", config.grpc_addr);

    // Create shared job queue
    let job_queue = Arc::new(RwLock::new(JobQueue::new()));

    // Create gRPC service
    let service = EtcScraperService::new(config.clone(), job_queue.clone());

    // Parse address
    let addr = config.grpc_addr.parse()?;

    // Start gRPC server
    Server::builder()
        .add_service(EtcScraperServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
