//! gRPC module
//!
//! Contains the generated gRPC code and service implementations.

pub mod gateway_server {
    pub mod proto {
        tonic::include_proto!("gateway");
    }
    pub use proto::*;
}

pub mod gateway_service;
pub mod scraper_service;

pub use scraper_service::EtcScraperService;
