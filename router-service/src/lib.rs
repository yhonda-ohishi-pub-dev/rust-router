//! ETC Scraper Router Service
//!
//! gRPC gateway and job management for ETC scraping operations.

pub mod config;
pub mod grpc;
pub mod job;
pub mod scraper;

/// Generated protobuf code
pub mod proto {
    tonic::include_proto!("scraper");
}

pub use config::RouterConfig;
pub use grpc::EtcScraperService;
pub use job::{AccountResult, JobQueue, JobState, JobStatus};
pub use scraper::{MockScraperService, ScrapeConfig, ScrapeResult, ScraperError, ScraperService};
