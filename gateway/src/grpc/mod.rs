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

// PDF generator proto
pub mod pdf_server {
    pub use proto::pdf::*;
}

pub mod gateway_service;
pub mod scraper_service;
pub mod pdf_service;

pub use scraper_service::EtcScraperService;
pub use pdf_service::PdfGeneratorService;
