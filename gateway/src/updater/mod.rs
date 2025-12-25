//! Auto-update functionality for the gateway service
//!
//! This module provides self-update capabilities, including:
//! - Version checking against a remote server
//! - Downloading new binaries
//! - Replacing the current binary and restarting

mod version;
mod downloader;
mod installer;

pub use version::{VersionChecker, VersionInfo};
pub use downloader::UpdateDownloader;
pub use installer::UpdateInstaller;

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during the update process
#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("Failed to check version: {0}")]
    VersionCheck(String),

    #[error("Failed to download update: {0}")]
    Download(String),

    #[error("Failed to install update: {0}")]
    Install(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("No update available")]
    NoUpdate,
}

/// Configuration for the auto-updater
#[derive(Clone, Debug)]
pub struct UpdateConfig {
    /// URL to check for updates (returns JSON with version info)
    pub version_check_url: String,

    /// Base URL for downloading updates
    pub download_base_url: String,

    /// Directory for temporary update files
    pub temp_dir: PathBuf,

    /// Current version of the application
    pub current_version: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            version_check_url: String::new(),
            download_base_url: String::new(),
            temp_dir: std::env::temp_dir().join("gateway-updates"),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Auto-updater that manages the entire update process
pub struct AutoUpdater {
    config: UpdateConfig,
    version_checker: VersionChecker,
    downloader: UpdateDownloader,
    installer: UpdateInstaller,
}

impl AutoUpdater {
    /// Create a new AutoUpdater with the given configuration
    pub fn new(config: UpdateConfig) -> Self {
        let version_checker = VersionChecker::new(config.version_check_url.clone());
        let downloader = UpdateDownloader::new(config.download_base_url.clone(), config.temp_dir.clone());
        let installer = UpdateInstaller::new();

        Self {
            config,
            version_checker,
            downloader,
            installer,
        }
    }

    /// Check if an update is available
    pub async fn check_for_update(&self) -> Result<Option<VersionInfo>, UpdateError> {
        let latest = self.version_checker.get_latest_version().await?;

        if self.is_newer_version(&latest.version) {
            Ok(Some(latest))
        } else {
            Ok(None)
        }
    }

    /// Download and install an update
    pub async fn update(&self) -> Result<(), UpdateError> {
        let version_info = self.check_for_update().await?
            .ok_or(UpdateError::NoUpdate)?;

        tracing::info!("Downloading update version {}", version_info.version);
        let update_path = self.downloader.download(&version_info).await?;

        tracing::info!("Installing update from {:?}", update_path);
        self.installer.install(&update_path).await?;

        Ok(())
    }

    /// Compare versions to check if the remote version is newer
    fn is_newer_version(&self, remote_version: &str) -> bool {
        use std::cmp::Ordering;

        let parse_version = |v: &str| -> Vec<u32> {
            v.trim_start_matches('v')
                .split('.')
                .filter_map(|s| s.parse().ok())
                .collect()
        };

        let current = parse_version(&self.config.current_version);
        let remote = parse_version(remote_version);

        for (c, r) in current.iter().zip(remote.iter()) {
            match c.cmp(r) {
                Ordering::Less => return true,
                Ordering::Greater => return false,
                Ordering::Equal => continue,
            }
        }

        remote.len() > current.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        let config = UpdateConfig {
            current_version: "1.0.0".to_string(),
            ..Default::default()
        };
        let updater = AutoUpdater::new(config);

        assert!(updater.is_newer_version("1.0.1"));
        assert!(updater.is_newer_version("1.1.0"));
        assert!(updater.is_newer_version("2.0.0"));
        assert!(!updater.is_newer_version("1.0.0"));
        assert!(!updater.is_newer_version("0.9.0"));
    }

    #[test]
    fn test_version_with_v_prefix() {
        let config = UpdateConfig {
            current_version: "v1.0.0".to_string(),
            ..Default::default()
        };
        let updater = AutoUpdater::new(config);

        assert!(updater.is_newer_version("v1.0.1"));
        assert!(updater.is_newer_version("1.0.1"));
    }
}
