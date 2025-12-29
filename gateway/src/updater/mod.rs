//! Auto-update functionality for the gateway service
//!
//! This module provides self-update capabilities, including:
//! - Version checking against GitHub Releases API
//! - Downloading new binaries with checksum verification
//! - Replacing the current binary and restarting
//!
//! ## Usage
//!
//! ```rust,ignore
//! use gateway::updater::{AutoUpdater, UpdateConfig, UpdateChannel};
//!
//! let config = UpdateConfig {
//!     github_owner: "yhonda-ohishi-pub-dev".to_string(),
//!     github_repo: "rust-router".to_string(),
//!     update_channel: UpdateChannel::Stable,
//!     ..Default::default()
//! };
//!
//! let updater = AutoUpdater::new(config);
//!
//! // Check for updates
//! if let Some(version) = updater.check_for_update().await? {
//!     println!("New version available: {}", version.version);
//!     updater.update().await?;
//! }
//! ```

mod version;
mod downloader;
mod installer;

pub use version::{VersionChecker, VersionInfo, UpdateChannel, GitHubRelease, GitHubAsset};
pub use downloader::UpdateDownloader;
pub use installer::{UpdateInstaller, ServiceStatus, check_service_status, check_service_ready_for_install};

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
    /// GitHub repository owner (e.g., "yhonda-ohishi-pub-dev")
    pub github_owner: String,

    /// GitHub repository name (e.g., "rust-router")
    pub github_repo: String,

    /// Update channel (stable or beta)
    pub update_channel: UpdateChannel,

    /// Prefer MSI installer over executable (Windows only)
    pub prefer_msi: bool,

    /// Directory for temporary update files
    pub temp_dir: PathBuf,

    /// Current version of the application
    pub current_version: String,

    // Legacy fields for backwards compatibility
    /// URL to check for updates (returns JSON with version info)
    /// Deprecated: Use github_owner and github_repo instead
    #[deprecated(note = "Use github_owner and github_repo instead")]
    pub version_check_url: String,

    /// Base URL for downloading updates
    /// Deprecated: Downloads are now fetched from GitHub Releases
    #[deprecated(note = "Downloads are now fetched from GitHub Releases")]
    pub download_base_url: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        #[allow(deprecated)]
        Self {
            github_owner: String::new(),
            github_repo: String::new(),
            update_channel: UpdateChannel::default(),
            prefer_msi: false,
            temp_dir: std::env::temp_dir().join("gateway-updates"),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            version_check_url: String::new(),
            download_base_url: String::new(),
        }
    }
}

impl UpdateConfig {
    /// Create a new UpdateConfig for GitHub Releases
    pub fn new_github(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            github_owner: owner.into(),
            github_repo: repo.into(),
            ..Default::default()
        }
    }

    /// Set the update channel
    pub fn with_channel(mut self, channel: UpdateChannel) -> Self {
        self.update_channel = channel;
        self
    }

    /// Prefer MSI installer over executable (Windows only)
    pub fn with_prefer_msi(mut self, prefer: bool) -> Self {
        self.prefer_msi = prefer;
        self
    }

    /// Set the temporary directory for downloads
    pub fn with_temp_dir(mut self, temp_dir: PathBuf) -> Self {
        self.temp_dir = temp_dir;
        self
    }

    /// Check if GitHub configuration is set
    pub fn is_github_configured(&self) -> bool {
        !self.github_owner.is_empty() && !self.github_repo.is_empty()
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
    #[allow(deprecated)]
    pub fn new(config: UpdateConfig) -> Self {
        let version_checker = if config.is_github_configured() {
            VersionChecker::new_github(
                config.github_owner.clone(),
                config.github_repo.clone(),
            )
            .with_channel(config.update_channel.clone())
            .with_prefer_msi(config.prefer_msi)
        } else {
            VersionChecker::new(config.version_check_url.clone())
        };

        let downloader = UpdateDownloader::new(
            config.download_base_url.clone(),
            config.temp_dir.clone(),
        );
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

    /// Get the latest version info without comparing
    pub async fn get_latest_version(&self) -> Result<VersionInfo, UpdateError> {
        self.version_checker.get_latest_version().await
    }

    /// List all available releases
    pub async fn list_releases(&self, include_prerelease: bool) -> Result<Vec<GitHubRelease>, UpdateError> {
        self.version_checker.list_releases(include_prerelease).await
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

    /// Download and install a specific version
    pub async fn update_to_version(&self, version_info: &VersionInfo) -> Result<(), UpdateError> {
        tracing::info!("Downloading version {}", version_info.version);
        let update_path = self.downloader.download(version_info).await?;

        tracing::info!("Installing update from {:?}", update_path);
        self.installer.install(&update_path).await?;

        Ok(())
    }

    /// Get version info for a specific tag
    pub async fn get_version_by_tag(&self, tag: &str) -> Result<VersionInfo, UpdateError> {
        self.version_checker.get_version_by_tag(tag).await
    }

    /// Download and install a specific version by tag
    pub async fn update_from_tag(&self, tag: &str) -> Result<(), UpdateError> {
        let version_info = self.get_version_by_tag(tag).await?;

        tracing::info!("Downloading version {} from tag {}", version_info.version, tag);
        let update_path = self.downloader.download(&version_info).await?;

        tracing::info!("Installing update from {:?}", update_path);
        self.installer.install(&update_path).await?;

        Ok(())
    }

    /// Get current version
    pub fn current_version(&self) -> &str {
        &self.config.current_version
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

/// Format update information for display
pub fn format_update_info(version: &VersionInfo, current: &str) -> String {
    let mut output = String::new();
    output.push_str(&format!("Current version: {}\n", current));
    output.push_str(&format!("Latest version:  {}\n", version.version));

    if let Some(ref notes) = version.release_notes {
        output.push_str("\nRelease notes:\n");
        for line in notes.lines().take(10) {
            output.push_str(&format!("  {}\n", line));
        }
        if notes.lines().count() > 10 {
            output.push_str("  ...(truncated)\n");
        }
    }

    output
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

    #[test]
    fn test_update_config_new_github() {
        let config = UpdateConfig::new_github("owner", "repo");
        assert_eq!(config.github_owner, "owner");
        assert_eq!(config.github_repo, "repo");
        assert!(config.is_github_configured());
    }

    #[test]
    fn test_update_config_with_channel() {
        let config = UpdateConfig::new_github("owner", "repo")
            .with_channel(UpdateChannel::Beta);
        assert_eq!(config.update_channel, UpdateChannel::Beta);
    }

    /// Test checking for updates against real GitHub API
    /// Run with: cargo test test_check_update_real --lib -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_check_update_real() {
        let config = UpdateConfig::new_github("yhonda-ohishi-pub-dev", "rust-router");
        let updater = AutoUpdater::new(config);

        match updater.get_latest_version().await {
            Ok(version) => {
                println!("Latest version: {}", version.version);
                println!("Download URL: {}", version.download_url);
                if let Some(notes) = &version.release_notes {
                    println!("Release notes: {}", notes);
                }
            }
            Err(e) => {
                panic!("Failed to check version: {:?}", e);
            }
        }
    }

    /// Test version comparison with real release
    /// Run with: cargo test test_update_available_real --lib -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_update_available_real() {
        // Simulate older version to test update detection
        let mut config = UpdateConfig::new_github("yhonda-ohishi-pub-dev", "rust-router");
        config.current_version = "0.0.1".to_string();
        let updater = AutoUpdater::new(config);

        match updater.check_for_update().await {
            Ok(Some(version)) => {
                println!("Update available: {} -> {}", updater.current_version(), version.version);
                println!("Download URL: {}", version.download_url);
            }
            Ok(None) => {
                println!("No update available (current: {})", updater.current_version());
            }
            Err(e) => {
                panic!("Failed to check for update: {:?}", e);
            }
        }
    }
}
