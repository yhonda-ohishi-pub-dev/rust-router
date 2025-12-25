//! Database connection pool management.

use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::time::Duration;

use crate::config::DbConfig;
use error::DatabaseError;

/// Type alias for MySQL connection pool.
pub type DbPool = MySqlPool;

/// Create a new database connection pool.
pub async fn create_pool(config: &DbConfig) -> Result<DbPool, DatabaseError> {
    tracing::info!(
        "Creating database pool: {}:{}/{}",
        config.host,
        config.port,
        config.database
    );

    let pool = MySqlPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
        .connect(&config.connection_url())
        .await
        .map_err(|e| {
            tracing::error!("Failed to create database pool: {}", e);
            DatabaseError::ConnectionFailed(e.to_string())
        })?;

    tracing::info!("Database pool created successfully");
    Ok(pool)
}

/// Check if the database connection is healthy.
pub async fn health_check(pool: &DbPool) -> Result<(), DatabaseError> {
    sqlx::query("SELECT 1")
        .execute(pool)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_url() {
        let config = DbConfig::new("localhost", 3306, "testdb", "user", "pass");
        assert_eq!(
            config.connection_url(),
            "mysql://user:pass@localhost:3306/testdb"
        );
    }
}
