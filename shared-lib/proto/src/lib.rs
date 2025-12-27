//! Shared protobuf definitions for all services
//!
//! This crate provides generated gRPC code from proto files.
//! Use feature flags to select which proto definitions to include:
//!
//! - `gateway`: Gateway service definitions
//! - `scraper`: ETC Scraper service definitions
//! - `timecard`: Timecard service definitions
//! - `all`: All proto definitions
//! - `reflection`: Enable gRPC reflection support

/// Gateway proto definitions
pub mod gateway {
    tonic::include_proto!("gateway");
}

/// Scraper proto definitions (front-compatible)
pub mod scraper {
    tonic::include_proto!("scraper");
}

/// PDF generator proto definitions
pub mod pdf {
    tonic::include_proto!("pdf");
}

// Re-export commonly used types for convenience
pub use gateway::*;

/// File descriptor set for gRPC reflection
#[cfg(feature = "reflection")]
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gateway_descriptor.bin"));
