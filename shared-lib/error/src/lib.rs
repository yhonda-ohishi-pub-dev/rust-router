//! Common error types for microservices.
//!
//! This crate provides unified error handling across all services.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Application-level errors.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Authentication error: {0}")]
    Auth(#[from] AuthError),

    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Authentication-related errors.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Invalid token")]
    InvalidToken,

    #[error("Token expired")]
    TokenExpired,

    #[error("Token creation failed")]
    TokenCreationFailed,

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden")]
    Forbidden,
}

/// Database-related errors.
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Record not found")]
    NotFound,

    #[error("Duplicate entry: {0}")]
    DuplicateEntry(String),

    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
}

/// Error response for API clients.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error code for programmatic handling
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Optional additional details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ErrorResponse {
    /// Create a new error response.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Add details to the error response.
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

impl From<AuthError> for ErrorResponse {
    fn from(err: AuthError) -> Self {
        let (code, message) = match &err {
            AuthError::InvalidCredentials => ("AUTH_INVALID_CREDENTIALS", "Invalid credentials"),
            AuthError::InvalidToken => ("AUTH_INVALID_TOKEN", "Invalid token"),
            AuthError::TokenExpired => ("AUTH_TOKEN_EXPIRED", "Token has expired"),
            AuthError::TokenCreationFailed => ("AUTH_TOKEN_CREATION_FAILED", "Failed to create token"),
            AuthError::Unauthorized => ("AUTH_UNAUTHORIZED", "Unauthorized"),
            AuthError::Forbidden => ("AUTH_FORBIDDEN", "Access forbidden"),
        };
        Self::new(code, message)
    }
}

impl From<DatabaseError> for ErrorResponse {
    fn from(err: DatabaseError) -> Self {
        let (code, message) = match &err {
            DatabaseError::ConnectionFailed(_) => ("DB_CONNECTION_FAILED", "Database connection failed"),
            DatabaseError::QueryFailed(_) => ("DB_QUERY_FAILED", "Database query failed"),
            DatabaseError::NotFound => ("DB_NOT_FOUND", "Record not found"),
            DatabaseError::DuplicateEntry(_) => ("DB_DUPLICATE_ENTRY", "Duplicate entry"),
            DatabaseError::TransactionFailed(_) => ("DB_TRANSACTION_FAILED", "Transaction failed"),
        };
        Self::new(code, message)
    }
}

/// Result type alias using AppError.
pub type Result<T> = std::result::Result<T, AppError>;
