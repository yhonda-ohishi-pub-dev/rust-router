//! Update download functionality

use super::{UpdateError, VersionInfo};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

/// Downloads updates from a remote server
pub struct UpdateDownloader {
    download_base_url: String,
    temp_dir: PathBuf,
    client: reqwest::Client,
}

impl UpdateDownloader {
    /// Create a new UpdateDownloader
    pub fn new(download_base_url: String, temp_dir: PathBuf) -> Self {
        Self {
            download_base_url,
            temp_dir,
            client: reqwest::Client::new(),
        }
    }

    /// Download an update and return the path to the downloaded file
    pub async fn download(&self, version_info: &VersionInfo) -> Result<PathBuf, UpdateError> {
        // Create temp directory if it doesn't exist
        tokio::fs::create_dir_all(&self.temp_dir).await?;

        // Determine download URL
        let download_url = if version_info.download_url.starts_with("http") {
            version_info.download_url.clone()
        } else {
            format!("{}/{}", self.download_base_url, version_info.download_url)
        };

        tracing::debug!("Downloading update from: {}", download_url);

        // Download the file
        let response = self.client
            .get(&download_url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::Download(
                format!("Server returned status: {}", response.status())
            ));
        }

        // Determine filename
        let filename = self.extract_filename(&download_url, &version_info.version);
        let download_path = self.temp_dir.join(&filename);

        // Write to file
        let bytes = response.bytes().await?;

        // Verify checksum if provided
        if let Some(ref expected_checksum) = version_info.checksum {
            let actual_checksum = self.calculate_sha256(&bytes);
            if &actual_checksum != expected_checksum {
                return Err(UpdateError::Download(
                    format!("Checksum mismatch: expected {}, got {}", expected_checksum, actual_checksum)
                ));
            }
            tracing::debug!("Checksum verified: {}", actual_checksum);
        }

        let mut file = tokio::fs::File::create(&download_path).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;

        tracing::info!("Update downloaded to {:?}", download_path);

        Ok(download_path)
    }

    /// Extract filename from URL or generate one based on version
    fn extract_filename(&self, url: &str, version: &str) -> String {
        url.rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                #[cfg(windows)]
                {
                    format!("gateway-{}.exe", version)
                }
                #[cfg(not(windows))]
                {
                    format!("gateway-{}", version)
                }
            })
    }

    /// Calculate SHA256 checksum of data
    fn calculate_sha256(&self, data: &[u8]) -> String {
        let hash = Sha256::digest(data);
        hex::encode(hash)
    }
}
