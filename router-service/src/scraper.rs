//! ScraperService trait and related types for integration with scraper-service.

use async_trait::async_trait;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during scraping operations
#[derive(Error, Debug)]
pub enum ScraperError {
    #[error("Browser initialization error: {0}")]
    BrowserInit(String),

    #[error("Navigation error: {0}")]
    Navigation(String),

    #[error("Login error: {0}")]
    Login(String),

    #[error("Download error: {0}")]
    Download(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("File I/O error: {0}")]
    FileIO(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Configuration for a scrape operation
#[derive(Debug, Clone)]
pub struct ScrapeConfig {
    /// User ID for login
    pub user_id: String,
    /// Password for login
    pub password: String,
    /// Display name for the account
    pub name: String,
    /// Download directory path
    pub download_path: PathBuf,
    /// Run in headless mode
    pub headless: bool,
}

/// Result of a successful scrape operation
#[derive(Debug, Clone)]
pub struct ScrapeResult {
    /// Path to the downloaded CSV file
    pub csv_path: PathBuf,
    /// CSV file content
    pub csv_content: Vec<u8>,
}

/// Trait for scraper service implementations.
///
/// This trait defines the interface that scraper-service must implement
/// for InProcess integration with the router.
#[async_trait]
pub trait ScraperService: Send + Sync {
    /// Execute a scrape operation for a single account
    async fn scrape(&self, config: ScrapeConfig) -> Result<ScrapeResult, ScraperError>;
}

/// Mock scraper service for testing and development
#[derive(Debug, Default)]
pub struct MockScraperService;

#[async_trait]
impl ScraperService for MockScraperService {
    async fn scrape(&self, config: ScrapeConfig) -> Result<ScrapeResult, ScraperError> {
        tracing::info!(
            "Mock scrape for account: {} ({})",
            config.name,
            config.user_id
        );

        // Simulate some work
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Create a mock CSV file
        let csv_content = format!(
            "date,card_number,entry_ic,exit_ic,amount\n\
             2024-01-01,1234-5678-9012-3456,Tokyo IC,Osaka IC,5000\n"
        );

        let csv_path = config
            .download_path
            .join(format!("{}_{}.csv", config.user_id, "mock"));

        // Create download directory if it doesn't exist
        if let Some(parent) = csv_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Write mock file
        tokio::fs::write(&csv_path, &csv_content).await?;

        Ok(ScrapeResult {
            csv_path,
            csv_content: csv_content.into_bytes(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_scraper() {
        let scraper = MockScraperService::default();
        let config = ScrapeConfig {
            user_id: "test_user".to_string(),
            password: "test_pass".to_string(),
            name: "Test User".to_string(),
            download_path: std::env::temp_dir().join("router-test"),
            headless: true,
        };

        let result = scraper.scrape(config).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        assert!(result.csv_path.exists());
        assert!(!result.csv_content.is_empty());

        // Cleanup
        let _ = std::fs::remove_file(&result.csv_path);
    }
}
