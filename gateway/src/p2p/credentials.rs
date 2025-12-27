//! P2P Credentials management
//!
//! Handles loading, saving, and managing API keys and refresh tokens
//! for P2P authentication.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Errors that can occur during credential operations
#[derive(Error, Debug)]
pub enum CredentialsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Credentials file not found: {0}")]
    NotFound(String),

    #[error("Invalid credentials format")]
    InvalidFormat,
}

/// P2P Credentials containing API key and optional refresh token
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct P2PCredentials {
    /// API key for authentication
    pub api_key: String,

    /// Application ID assigned by the server
    #[serde(default)]
    pub app_id: String,

    /// Refresh token for obtaining new API keys
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl P2PCredentials {
    /// Create new credentials with API key only
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            app_id: String::new(),
            refresh_token: None,
        }
    }

    /// Create credentials with all fields
    pub fn with_refresh_token(api_key: String, app_id: String, refresh_token: String) -> Self {
        Self {
            api_key,
            app_id,
            refresh_token: Some(refresh_token),
        }
    }

    /// Load credentials from a file
    ///
    /// Supports two formats:
    /// 1. ENV format: `P2P_API_KEY=xxx`, `P2P_APP_ID=xxx`, `P2P_REFRESH_TOKEN=xxx`
    /// 2. JSON format: `{"api_key": "xxx", "app_id": "xxx", "refresh_token": "xxx"}`
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, CredentialsError> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(CredentialsError::NotFound(
                path.display().to_string(),
            ));
        }

        let content = std::fs::read_to_string(path)?;

        // Try JSON format first
        if content.trim().starts_with('{') {
            return serde_json::from_str(&content)
                .map_err(|e| CredentialsError::Parse(e.to_string()));
        }

        // Parse ENV format
        Self::parse_env_format(&content)
    }

    /// Parse ENV format credentials
    fn parse_env_format(content: &str) -> Result<Self, CredentialsError> {
        let mut api_key = None;
        let mut app_id = String::new();
        let mut refresh_token = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');

                match key {
                    "P2P_API_KEY" | "API_KEY" => api_key = Some(value.to_string()),
                    "P2P_APP_ID" | "APP_ID" => app_id = value.to_string(),
                    "P2P_REFRESH_TOKEN" | "REFRESH_TOKEN" => {
                        if !value.is_empty() {
                            refresh_token = Some(value.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        let api_key = api_key.ok_or(CredentialsError::InvalidFormat)?;

        Ok(Self {
            api_key,
            app_id,
            refresh_token,
        })
    }

    /// Save credentials to a file in ENV format
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), CredentialsError> {
        let path = path.as_ref();

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut content = format!("P2P_API_KEY={}\n", self.api_key);

        if !self.app_id.is_empty() {
            content.push_str(&format!("P2P_APP_ID={}\n", self.app_id));
        }

        if let Some(ref token) = self.refresh_token {
            content.push_str(&format!("P2P_REFRESH_TOKEN={}\n", token));
        }

        std::fs::write(path, content)?;

        Ok(())
    }

    /// Save credentials as JSON
    pub fn save_json<P: AsRef<Path>>(&self, path: P) -> Result<(), CredentialsError> {
        let path = path.as_ref();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CredentialsError::Parse(e.to_string()))?;

        std::fs::write(path, content)?;

        Ok(())
    }

    /// Get default credentials file path
    /// Uses C:\ProgramData\Gateway on Windows for service compatibility
    pub fn default_path() -> std::path::PathBuf {
        Self::service_path()
    }

    /// Get service-compatible credentials path (C:\ProgramData\Gateway on Windows)
    #[cfg(windows)]
    pub fn service_path() -> std::path::PathBuf {
        std::path::PathBuf::from(r"C:\ProgramData\Gateway")
            .join("p2p_credentials.env")
    }

    #[cfg(not(windows))]
    pub fn service_path() -> std::path::PathBuf {
        std::path::PathBuf::from("/etc/gateway")
            .join("p2p_credentials.env")
    }

    /// Get user-specific credentials path (for backwards compatibility)
    #[allow(dead_code)]
    pub fn user_path() -> std::path::PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("gateway")
            .join("p2p_credentials.env")
    }

    /// Check if credentials have a refresh token
    pub fn has_refresh_token(&self) -> bool {
        self.refresh_token.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_env_format() {
        let content = r#"
P2P_API_KEY=test-api-key
P2P_APP_ID=app-123
P2P_REFRESH_TOKEN=refresh-token-456
"#;

        let creds = P2PCredentials::parse_env_format(content).unwrap();
        assert_eq!(creds.api_key, "test-api-key");
        assert_eq!(creds.app_id, "app-123");
        assert_eq!(creds.refresh_token, Some("refresh-token-456".to_string()));
    }

    #[test]
    fn test_parse_env_format_minimal() {
        let content = "P2P_API_KEY=only-key";

        let creds = P2PCredentials::parse_env_format(content).unwrap();
        assert_eq!(creds.api_key, "only-key");
        assert!(creds.app_id.is_empty());
        assert!(creds.refresh_token.is_none());
    }

    #[test]
    fn test_load_json_format() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"api_key": "json-key", "app_id": "json-app", "refresh_token": "json-token"}}"#
        )
        .unwrap();

        let creds = P2PCredentials::load(file.path()).unwrap();
        assert_eq!(creds.api_key, "json-key");
        assert_eq!(creds.app_id, "json-app");
        assert_eq!(creds.refresh_token, Some("json-token".to_string()));
    }

    #[test]
    fn test_save_and_load() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        drop(file);

        let creds = P2PCredentials::with_refresh_token(
            "save-key".to_string(),
            "save-app".to_string(),
            "save-token".to_string(),
        );

        creds.save(&path).unwrap();

        let loaded = P2PCredentials::load(&path).unwrap();
        assert_eq!(loaded.api_key, creds.api_key);
        assert_eq!(loaded.app_id, creds.app_id);
        assert_eq!(loaded.refresh_token, creds.refresh_token);

        std::fs::remove_file(&path).ok();
    }
}
