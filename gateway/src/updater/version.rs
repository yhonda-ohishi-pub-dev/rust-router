//! Version checking functionality

use super::UpdateError;
use serde::Deserialize;

/// Information about a specific version
#[derive(Clone, Debug, Deserialize)]
pub struct VersionInfo {
    /// Version string (e.g., "1.2.3" or "v1.2.3")
    pub version: String,

    /// Download URL for this version
    pub download_url: String,

    /// SHA256 checksum of the binary
    #[serde(default)]
    pub checksum: Option<String>,

    /// Release notes or changelog
    #[serde(default)]
    pub release_notes: Option<String>,

    /// Whether this is a mandatory update
    #[serde(default)]
    pub mandatory: bool,
}

/// Checks for available updates from a remote server
pub struct VersionChecker {
    version_check_url: String,
    client: reqwest::Client,
}

impl VersionChecker {
    /// Create a new VersionChecker
    pub fn new(version_check_url: String) -> Self {
        Self {
            version_check_url,
            client: reqwest::Client::new(),
        }
    }

    /// Get the latest version information from the server
    pub async fn get_latest_version(&self) -> Result<VersionInfo, UpdateError> {
        if self.version_check_url.is_empty() {
            return Err(UpdateError::VersionCheck("Version check URL not configured".to_string()));
        }

        let response = self.client
            .get(&self.version_check_url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::VersionCheck(
                format!("Server returned status: {}", response.status())
            ));
        }

        let version_info: VersionInfo = response.json().await
            .map_err(|e| UpdateError::VersionCheck(format!("Failed to parse version info: {}", e)))?;

        Ok(version_info)
    }
}
