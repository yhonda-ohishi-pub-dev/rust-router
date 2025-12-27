//! Version checking functionality with GitHub Releases API support

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

/// GitHub Release asset information
#[derive(Clone, Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    pub content_type: String,
}

/// GitHub Release information
#[derive(Clone, Debug, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<GitHubAsset>,
    pub published_at: Option<String>,
}

/// Update channel for release selection
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateChannel {
    /// Stable releases only (default)
    #[default]
    Stable,
    /// Include beta/pre-releases
    Beta,
}

impl std::fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateChannel::Stable => write!(f, "stable"),
            UpdateChannel::Beta => write!(f, "beta"),
        }
    }
}

impl std::str::FromStr for UpdateChannel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stable" => Ok(UpdateChannel::Stable),
            "beta" => Ok(UpdateChannel::Beta),
            _ => Err(format!("Invalid update channel: {}. Use 'stable' or 'beta'", s)),
        }
    }
}

/// Checks for available updates from GitHub Releases
pub struct VersionChecker {
    github_owner: String,
    github_repo: String,
    update_channel: UpdateChannel,
    client: reqwest::Client,
    /// Legacy URL for backwards compatibility
    version_check_url: Option<String>,
}

impl VersionChecker {
    /// Create a new VersionChecker for GitHub Releases
    pub fn new_github(github_owner: String, github_repo: String) -> Self {
        Self {
            github_owner,
            github_repo,
            update_channel: UpdateChannel::default(),
            client: reqwest::Client::new(),
            version_check_url: None,
        }
    }

    /// Create a new VersionChecker with legacy URL (backwards compatibility)
    pub fn new(version_check_url: String) -> Self {
        Self {
            github_owner: String::new(),
            github_repo: String::new(),
            update_channel: UpdateChannel::default(),
            client: reqwest::Client::new(),
            version_check_url: Some(version_check_url),
        }
    }

    /// Set the update channel
    pub fn with_channel(mut self, channel: UpdateChannel) -> Self {
        self.update_channel = channel;
        self
    }

    /// Get the latest version information from GitHub Releases
    pub async fn get_latest_version(&self) -> Result<VersionInfo, UpdateError> {
        // Use legacy URL if configured (backwards compatibility)
        if let Some(ref url) = self.version_check_url {
            if !url.is_empty() {
                return self.get_latest_version_legacy(url).await;
            }
        }

        // Use GitHub Releases API
        self.get_latest_version_github().await
    }

    /// Get latest version from GitHub Releases API
    async fn get_latest_version_github(&self) -> Result<VersionInfo, UpdateError> {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            return Err(UpdateError::VersionCheck(
                "GitHub owner/repo not configured".to_string()
            ));
        }

        let release = match self.update_channel {
            UpdateChannel::Stable => self.get_latest_stable_release().await?,
            UpdateChannel::Beta => self.get_latest_release_including_prerelease().await?,
        };

        // Find the appropriate asset for this platform
        let asset = self.select_asset(&release)?;

        // Try to get checksum file
        let checksum = self.get_checksum(&release, &asset.name).await.ok();

        Ok(VersionInfo {
            version: release.tag_name.clone(),
            download_url: asset.browser_download_url.clone(),
            checksum,
            release_notes: release.body.clone(),
            mandatory: false,
        })
    }

    /// Get the latest stable release (excludes pre-releases)
    async fn get_latest_stable_release(&self) -> Result<GitHubRelease, UpdateError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            self.github_owner, self.github_repo
        );

        self.fetch_release(&url).await
    }

    /// Get the latest release including pre-releases
    async fn get_latest_release_including_prerelease(&self) -> Result<GitHubRelease, UpdateError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases",
            self.github_owner, self.github_repo
        );

        let response = self.client
            .get(&url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::VersionCheck(
                format!("GitHub API returned status: {}", response.status())
            ));
        }

        let releases: Vec<GitHubRelease> = response.json().await
            .map_err(|e| UpdateError::VersionCheck(format!("Failed to parse releases: {}", e)))?;

        // Get the first non-draft release
        releases.into_iter()
            .find(|r| !r.draft)
            .ok_or_else(|| UpdateError::VersionCheck("No releases found".to_string()))
    }

    /// Fetch a single release from the given URL
    async fn fetch_release(&self, url: &str) -> Result<GitHubRelease, UpdateError> {
        let response = self.client
            .get(url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::VersionCheck(
                format!("GitHub API returned status: {}", response.status())
            ));
        }

        response.json().await
            .map_err(|e| UpdateError::VersionCheck(format!("Failed to parse release: {}", e)))
    }

    /// Select the appropriate asset for the current platform
    fn select_asset<'a>(&self, release: &'a GitHubRelease) -> Result<&'a GitHubAsset, UpdateError> {
        let (os, arch) = get_platform_info();

        // Expected filename patterns:
        // gateway-v1.0.0-windows-x86_64.exe
        // gateway-v1.0.0-linux-x86_64
        // gateway-v1.0.0-macos-x86_64
        // gateway-v1.0.0-macos-aarch64

        let patterns = [
            format!("gateway-{}-{}-{}", release.tag_name, os, arch),
            format!("gateway-{}-{}", os, arch),
            format!("gateway-{}", os),
        ];

        // Add .exe suffix for Windows patterns
        let patterns: Vec<String> = if os == "windows" {
            patterns.iter().map(|p| format!("{}.exe", p)).collect()
        } else {
            patterns.to_vec()
        };

        for asset in &release.assets {
            let name_lower = asset.name.to_lowercase();
            for pattern in &patterns {
                if name_lower.contains(&pattern.to_lowercase()) {
                    return Ok(asset);
                }
            }
        }

        // Fallback: try to find any matching OS
        for asset in &release.assets {
            let name_lower = asset.name.to_lowercase();
            if name_lower.contains(&os) {
                return Ok(asset);
            }
        }

        Err(UpdateError::VersionCheck(format!(
            "No suitable asset found for {}-{}. Available assets: {:?}",
            os, arch,
            release.assets.iter().map(|a| &a.name).collect::<Vec<_>>()
        )))
    }

    /// Try to get the SHA256 checksum for an asset
    async fn get_checksum(&self, release: &GitHubRelease, asset_name: &str) -> Result<String, UpdateError> {
        // Look for a .sha256 file
        let checksum_filename = format!("{}.sha256", asset_name);

        let checksum_asset = release.assets.iter()
            .find(|a| a.name == checksum_filename)
            .ok_or_else(|| UpdateError::VersionCheck("Checksum file not found".to_string()))?;

        let response = self.client
            .get(&checksum_asset.browser_download_url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::VersionCheck(
                format!("Failed to download checksum: {}", response.status())
            ));
        }

        let content = response.text().await
            .map_err(|e| UpdateError::VersionCheck(format!("Failed to read checksum: {}", e)))?;

        // Checksum file format: "hash  filename" or just "hash"
        Ok(content.split_whitespace().next().unwrap_or("").to_string())
    }

    /// Legacy version check (backwards compatibility)
    async fn get_latest_version_legacy(&self, url: &str) -> Result<VersionInfo, UpdateError> {
        let response = self.client
            .get(url)
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

    /// Get all available releases (for listing)
    pub async fn list_releases(&self, include_prerelease: bool) -> Result<Vec<GitHubRelease>, UpdateError> {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            return Err(UpdateError::VersionCheck(
                "GitHub owner/repo not configured".to_string()
            ));
        }

        let url = format!(
            "https://api.github.com/repos/{}/{}/releases",
            self.github_owner, self.github_repo
        );

        let response = self.client
            .get(&url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::VersionCheck(
                format!("GitHub API returned status: {}", response.status())
            ));
        }

        let releases: Vec<GitHubRelease> = response.json().await
            .map_err(|e| UpdateError::VersionCheck(format!("Failed to parse releases: {}", e)))?;

        Ok(releases.into_iter()
            .filter(|r| !r.draft && (include_prerelease || !r.prerelease))
            .collect())
    }
}

/// Get the current platform information (OS, architecture)
fn get_platform_info() -> (String, String) {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "unknown"
    };

    (os.to_string(), arch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_info() {
        let (os, arch) = get_platform_info();
        assert!(!os.is_empty());
        assert!(!arch.is_empty());

        #[cfg(windows)]
        assert_eq!(os, "windows");

        #[cfg(target_arch = "x86_64")]
        assert_eq!(arch, "x86_64");
    }

    #[test]
    fn test_update_channel_parse() {
        assert_eq!("stable".parse::<UpdateChannel>().unwrap(), UpdateChannel::Stable);
        assert_eq!("beta".parse::<UpdateChannel>().unwrap(), UpdateChannel::Beta);
        assert_eq!("STABLE".parse::<UpdateChannel>().unwrap(), UpdateChannel::Stable);
        assert!("invalid".parse::<UpdateChannel>().is_err());
    }

    #[test]
    fn test_update_channel_display() {
        assert_eq!(UpdateChannel::Stable.to_string(), "stable");
        assert_eq!(UpdateChannel::Beta.to_string(), "beta");
    }

    #[tokio::test]
    async fn test_version_checker_no_config() {
        let checker = VersionChecker::new_github(String::new(), String::new());
        let result = checker.get_latest_version().await;
        assert!(result.is_err());
    }
}
