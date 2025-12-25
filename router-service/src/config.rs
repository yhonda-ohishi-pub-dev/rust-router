use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Router service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// gRPC server address
    pub grpc_addr: String,

    /// Default download path for scraped files
    pub download_path: PathBuf,

    /// Maximum concurrent scrape jobs
    pub max_concurrent_jobs: usize,

    /// Job timeout in seconds
    pub job_timeout_secs: u64,

    /// Delay between accounts in seconds (to avoid rate limiting)
    pub account_delay_secs: u64,

    /// Run browser in headless mode by default
    pub default_headless: bool,

    /// Service version
    pub version: String,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            grpc_addr: "[::1]:50051".to_string(),
            download_path: PathBuf::from("./downloads"),
            max_concurrent_jobs: 1,
            job_timeout_secs: 300,
            account_delay_secs: 2,
            default_headless: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl RouterConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(addr) = std::env::var("GRPC_ADDR") {
            config.grpc_addr = addr;
        }

        if let Ok(path) = std::env::var("DOWNLOAD_PATH") {
            config.download_path = PathBuf::from(path);
        }

        if let Ok(max_jobs) = std::env::var("MAX_CONCURRENT_JOBS") {
            if let Ok(n) = max_jobs.parse() {
                config.max_concurrent_jobs = n;
            }
        }

        if let Ok(timeout) = std::env::var("JOB_TIMEOUT_SECS") {
            if let Ok(n) = timeout.parse() {
                config.job_timeout_secs = n;
            }
        }

        if let Ok(delay) = std::env::var("ACCOUNT_DELAY_SECS") {
            if let Ok(n) = delay.parse() {
                config.account_delay_secs = n;
            }
        }

        if let Ok(headless) = std::env::var("DEFAULT_HEADLESS") {
            config.default_headless = headless.to_lowercase() == "true" || headless == "1";
        }

        config
    }

    /// Get job timeout as Duration
    pub fn job_timeout(&self) -> Duration {
        Duration::from_secs(self.job_timeout_secs)
    }

    /// Get account delay as Duration
    pub fn account_delay(&self) -> Duration {
        Duration::from_secs(self.account_delay_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RouterConfig::default();
        assert_eq!(config.grpc_addr, "[::1]:50051");
        assert_eq!(config.max_concurrent_jobs, 1);
        assert!(config.default_headless);
    }
}
