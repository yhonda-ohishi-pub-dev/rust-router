//! P2P OAuth Authentication
//!
//! Implements OAuth setup flow for P2P authentication using polling method.
//! Compatible with cf-wbrtc-auth server.

use crate::p2p::credentials::{CredentialsError, P2PCredentials};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during OAuth setup
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Setup expired or cancelled")]
    SetupExpired,

    #[error("Setup failed: {0}")]
    SetupFailed(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Browser launch failed: {0}")]
    BrowserLaunch(String),

    #[error("Credentials error: {0}")]
    Credentials(#[from] CredentialsError),

    #[error("Refresh failed: {0}")]
    RefreshFailed(String),
}

/// Configuration for OAuth setup
#[derive(Clone, Debug)]
pub struct SetupConfig {
    /// Auth server base URL (e.g., "https://cf-wbrtc-auth.example.com")
    pub auth_server_url: String,

    /// Application name to display during auth
    pub app_name: String,

    /// Polling interval in seconds (default: 2)
    pub poll_interval_secs: u64,

    /// Maximum polling duration in seconds (default: 300 = 5 minutes)
    pub timeout_secs: u64,

    /// Whether to automatically open browser
    pub auto_open_browser: bool,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            auth_server_url: String::new(),
            app_name: "Gateway".to_string(),
            poll_interval_secs: 2,
            timeout_secs: 300,
            auto_open_browser: true,
        }
    }
}

/// Response from setup initiation
#[derive(Debug, Deserialize)]
struct SetupInitResponse {
    /// Setup session token
    token: String,

    /// URL for user to visit
    url: String,
}

/// Response from setup polling
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupPollResponse {
    /// Status: "pending", "complete", "expired", "error"
    status: String,

    /// API key (only present when status is "complete")
    api_key: Option<String>,

    /// App ID (only present when status is "complete")
    app_id: Option<String>,

    /// Refresh token (only present when status is "complete")
    refresh_token: Option<String>,

    /// Error message (only present when status is "error")
    #[serde(default)]
    error: Option<String>,
}

/// Response from token refresh
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshResponse {
    api_key: String,
    app_id: String,
    refresh_token: String,
}

/// Request for token refresh
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshRequest {
    refresh_token: String,
}

/// OAuth Setup Handler
pub struct OAuthSetup {
    client: Client,
    config: SetupConfig,
}

impl OAuthSetup {
    /// Create a new OAuth setup handler
    pub fn new(config: SetupConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, config }
    }

    /// Perform OAuth setup flow
    ///
    /// 1. Initiate setup session with auth server
    /// 2. Display/open auth URL for user
    /// 3. Poll for completion
    /// 4. Return credentials on success
    pub async fn setup(&self) -> Result<P2PCredentials, AuthError> {
        // Step 1: Initiate setup
        let init_response = self.initiate_setup().await?;

        tracing::info!(
            "OAuth setup initiated. Please authenticate at: {}",
            init_response.url
        );

        // Step 2: Open browser if configured
        if self.config.auto_open_browser {
            if let Err(e) = open::that(&init_response.url) {
                tracing::warn!("Failed to open browser: {}. Please open the URL manually.", e);
            }
        }

        // Step 3: Poll for completion
        let credentials = self.poll_for_completion(&init_response.token).await?;

        tracing::info!("OAuth setup completed successfully");

        Ok(credentials)
    }

    /// Initiate setup session
    async fn initiate_setup(&self) -> Result<SetupInitResponse, AuthError> {
        let url = format!("{}/setup/init", self.config.auth_server_url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "app_name": self.config.app_name
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AuthError::SetupFailed(format!(
                "Server returned {}: {}",
                status, body
            )));
        }

        response
            .json()
            .await
            .map_err(|e| AuthError::InvalidResponse(e.to_string()))
    }

    /// Poll for setup completion
    async fn poll_for_completion(&self, setup_id: &str) -> Result<P2PCredentials, AuthError> {
        let url = format!("{}/setup/poll?token={}", self.config.auth_server_url, setup_id);
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let timeout = Duration::from_secs(self.config.timeout_secs);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(AuthError::SetupExpired);
            }

            tokio::time::sleep(poll_interval).await;

            let response = self.client.get(&url).send().await?;

            if !response.status().is_success() {
                continue;
            }

            let poll_response: SetupPollResponse = response
                .json()
                .await
                .map_err(|e| AuthError::InvalidResponse(e.to_string()))?;

            match poll_response.status.as_str() {
                "pending" => {
                    tracing::debug!("Setup still pending, continuing to poll...");
                    continue;
                }
                "complete" => {
                    let api_key = poll_response
                        .api_key
                        .ok_or_else(|| AuthError::InvalidResponse("Missing api_key".to_string()))?;
                    let app_id = poll_response.app_id.unwrap_or_default();
                    let refresh_token = poll_response.refresh_token;

                    return Ok(P2PCredentials {
                        api_key,
                        app_id,
                        refresh_token,
                    });
                }
                "expired" => {
                    return Err(AuthError::SetupExpired);
                }
                "error" => {
                    let error = poll_response.error.unwrap_or_else(|| "Unknown error".to_string());
                    return Err(AuthError::SetupFailed(error));
                }
                other => {
                    return Err(AuthError::InvalidResponse(format!(
                        "Unknown status: {}",
                        other
                    )));
                }
            }
        }
    }

    /// Refresh API key using refresh token
    ///
    /// Sends refresh token in JSON body to /api/app/refresh
    pub async fn refresh_api_key(
        &self,
        refresh_token: &str,
    ) -> Result<P2PCredentials, AuthError> {
        let url = format!("{}/api/app/refresh", self.config.auth_server_url);

        let response = self
            .client
            .post(&url)
            .json(&RefreshRequest {
                refresh_token: refresh_token.to_string(),
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AuthError::RefreshFailed(format!(
                "Server returned {}: {}",
                status, body
            )));
        }

        let refresh_response: RefreshResponse = response
            .json()
            .await
            .map_err(|e| AuthError::InvalidResponse(e.to_string()))?;

        Ok(P2PCredentials {
            api_key: refresh_response.api_key,
            app_id: refresh_response.app_id,
            refresh_token: Some(refresh_response.refresh_token),
        })
    }
}

/// Perform OAuth setup with configuration
pub async fn setup(config: SetupConfig) -> Result<P2PCredentials, AuthError> {
    let handler = OAuthSetup::new(config);
    handler.setup().await
}

/// Load credentials or perform setup if not found
pub async fn load_or_setup(
    credentials_path: Option<&str>,
    setup_config: SetupConfig,
) -> Result<P2PCredentials, AuthError> {
    let path = credentials_path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(P2PCredentials::default_path);

    // Try to load existing credentials
    match P2PCredentials::load(&path) {
        Ok(creds) => {
            tracing::info!("Loaded credentials from {}", path.display());
            Ok(creds)
        }
        Err(CredentialsError::NotFound(_)) => {
            tracing::info!("Credentials not found, starting OAuth setup...");

            let creds = setup(setup_config).await?;

            // Save credentials
            creds.save(&path)?;
            tracing::info!("Credentials saved to {}", path.display());

            Ok(creds)
        }
        Err(e) => Err(AuthError::Credentials(e)),
    }
}

/// Refresh credentials if they have a refresh token
pub async fn refresh_if_needed(
    credentials: &P2PCredentials,
    auth_server_url: &str,
) -> Result<P2PCredentials, AuthError> {
    if let Some(ref refresh_token) = credentials.refresh_token {
        let config = SetupConfig {
            auth_server_url: auth_server_url.to_string(),
            ..Default::default()
        };

        let handler = OAuthSetup::new(config);
        handler.refresh_api_key(refresh_token).await
    } else {
        Err(AuthError::RefreshFailed("No refresh token available".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_config_default() {
        let config = SetupConfig::default();

        assert!(config.auth_server_url.is_empty());
        assert_eq!(config.app_name, "Gateway");
        assert_eq!(config.poll_interval_secs, 2);
        assert_eq!(config.timeout_secs, 300);
        assert!(config.auto_open_browser);
    }

    #[test]
    fn test_setup_config_custom() {
        let config = SetupConfig {
            auth_server_url: "https://auth.example.com".to_string(),
            app_name: "TestApp".to_string(),
            poll_interval_secs: 5,
            timeout_secs: 600,
            auto_open_browser: false,
        };

        assert_eq!(config.auth_server_url, "https://auth.example.com");
        assert_eq!(config.app_name, "TestApp");
        assert_eq!(config.poll_interval_secs, 5);
        assert_eq!(config.timeout_secs, 600);
        assert!(!config.auto_open_browser);
    }

    #[test]
    fn test_oauth_setup_new() {
        let config = SetupConfig {
            auth_server_url: "https://auth.example.com".to_string(),
            ..Default::default()
        };

        let setup = OAuthSetup::new(config.clone());
        assert_eq!(setup.config.auth_server_url, "https://auth.example.com");
        assert_eq!(setup.config.app_name, "Gateway");
    }

    #[test]
    fn test_auth_error_display() {
        let err = AuthError::SetupExpired;
        assert_eq!(err.to_string(), "Setup expired or cancelled");

        let err = AuthError::SetupFailed("test error".to_string());
        assert_eq!(err.to_string(), "Setup failed: test error");

        let err = AuthError::InvalidResponse("bad json".to_string());
        assert_eq!(err.to_string(), "Invalid response: bad json");

        let err = AuthError::BrowserLaunch("not found".to_string());
        assert_eq!(err.to_string(), "Browser launch failed: not found");

        let err = AuthError::RefreshFailed("token expired".to_string());
        assert_eq!(err.to_string(), "Refresh failed: token expired");
    }

    #[tokio::test]
    async fn test_refresh_if_needed_no_token() {
        let creds = P2PCredentials {
            api_key: "test-key".to_string(),
            app_id: "test-app".to_string(),
            refresh_token: None,
        };

        let result = refresh_if_needed(&creds, "https://auth.example.com").await;

        assert!(result.is_err());
        match result {
            Err(AuthError::RefreshFailed(msg)) => {
                assert!(msg.contains("No refresh token"));
            }
            _ => panic!("Expected RefreshFailed error"),
        }
    }

    /// Integration test with real server
    /// Run with: cargo test test_real_refresh --lib -- --ignored
    #[tokio::test]
    #[ignore]
    async fn test_real_refresh() {
        // Load credentials from file
        let creds_path = std::path::PathBuf::from("p2p_credentials.env");
        let creds = P2PCredentials::load(&creds_path).expect("Failed to load credentials");

        println!("Loaded credentials:");
        println!("  API_KEY: {}...", &creds.api_key[..20]);
        println!("  APP_ID: {}", creds.app_id);
        println!(
            "  REFRESH_TOKEN: {}...",
            creds.refresh_token.as_ref().map(|t| &t[..20]).unwrap_or("None")
        );

        // Test refresh with real server
        let auth_server_url = "https://cf-wbrtc-auth.m-tama-ramu.workers.dev";
        let result = refresh_if_needed(&creds, auth_server_url).await;

        match result {
            Ok(new_creds) => {
                println!("\nRefresh successful!");
                println!("  New API_KEY: {}...", &new_creds.api_key[..20]);
                println!("  New APP_ID: {}", new_creds.app_id);
                assert!(!new_creds.api_key.is_empty());
                assert!(!new_creds.app_id.is_empty());
            }
            Err(e) => {
                panic!("Refresh failed: {:?}", e);
            }
        }
    }
}
