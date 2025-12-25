//! Gateway library for InProcess service calls
//!
//! This module exposes the gateway functionality as a library,
//! enabling InProcess calls from other services.

pub mod config;
pub mod grpc;
pub mod job;
pub mod p2p;
pub mod router;
pub mod scraper;
pub mod services;
pub mod updater;

pub use config::GatewayConfig;
pub use grpc::EtcScraperService;
pub use job::{AccountResult, JobQueue, JobState, JobStatus};
pub use p2p::{P2PConfig, P2PError, P2PManager};
pub use router::ServiceRouter;
pub use scraper::{MockScraperService, ScrapeConfig, ScrapeResult, ScraperError, ScraperService};
pub use updater::{AutoUpdater, UpdateConfig, UpdateError};
