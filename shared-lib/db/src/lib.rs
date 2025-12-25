//! Database utilities and connection pooling for microservices.
//!
//! This crate provides MySQL connection pool management using sqlx.

mod pool;
mod config;

pub use pool::{create_pool, DbPool};
pub use config::DbConfig;

// Re-export sqlx types for convenience
pub use sqlx::{self, MySql, Row};
