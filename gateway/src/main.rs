//! Gateway main entry point
//!
//! This is the gRPC gateway that receives external requests
//! and routes them to internal services via InProcess calls.

use std::net::SocketAddr;

use tonic::transport::Server;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod grpc;
mod router;
mod services;

use grpc::gateway_service::GatewayServiceImpl;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Configure address
    let addr: SocketAddr = "[::]:50051".parse()?;
    info!("Starting gateway on {}", addr);

    // Create gRPC service
    let gateway_service = GatewayServiceImpl::new();

    // Start gRPC server
    Server::builder()
        .add_service(grpc::gateway_server::gateway_service_server::GatewayServiceServer::new(gateway_service))
        .serve(addr)
        .await?;

    Ok(())
}
