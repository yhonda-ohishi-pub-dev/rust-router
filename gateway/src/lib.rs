//! Gateway library for InProcess service calls
//!
//! This module exposes the gateway functionality as a library,
//! enabling InProcess calls from other services.

pub mod grpc;
pub mod router;
pub mod services;

pub use router::ServiceRouter;

/// Gateway configuration
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub grpc_port: u16,
    pub enable_reflection: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            grpc_port: 50051,
            enable_reflection: true,
        }
    }
}
