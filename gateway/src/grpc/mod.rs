//! gRPC module
//!
//! Contains the generated gRPC code and service implementations.

// Re-export proto types from the shared proto crate
pub mod gateway_server {
    pub use proto::gateway::*;
}

// Scraper proto (front-compatible)
pub mod scraper_server {
    pub use proto::scraper::*;
}

pub mod gateway_service;
pub mod scraper_service;

pub use scraper_service::EtcScraperService;
