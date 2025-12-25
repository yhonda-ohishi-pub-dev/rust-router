//! Timecard Service
//!
//! This crate provides timecard management functionality.
//! It exposes services via tower::Service for InProcess calls from the gateway.

pub mod models;
pub mod repository;
pub mod service;

pub use models::{Timecard, TimecardEntry};
pub use service::TimecardService;

/// Service configuration
#[derive(Debug, Clone)]
pub struct TimecardConfig {
    pub database_url: String,
}

impl Default for TimecardConfig {
    fn default() -> Self {
        Self {
            database_url: String::from("mysql://localhost/timecard"),
        }
    }
}
