//! JWT encoding and decoding utilities.

use error::AuthError;
use hmac::{Hmac, Mac};
use jwt::{SignWithKey, VerifyWithKey};
use sha2::Sha256;
use std::collections::BTreeMap;

use crate::claims::{Claims, Role};

type HmacSha256 = Hmac<Sha256>;

/// JWT configuration.
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// Secret key for signing tokens
    pub secret: String,
    /// Token issuer
    pub issuer: String,
    /// Token validity duration in seconds
    pub expires_in_secs: i64,
}

impl JwtConfig {
    /// Create a new JWT configuration.
    pub fn new(secret: impl Into<String>, issuer: impl Into<String>, expires_in_secs: i64) -> Self {
        Self {
            secret: secret.into(),
            issuer: issuer.into(),
            expires_in_secs,
        }
    }
}

/// Encode claims into a JWT token.
pub fn encode_token(claims: &Claims, secret: &str) -> Result<String, AuthError> {
    let key = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| {
        tracing::error!("Failed to create HMAC key: {}", e);
        AuthError::TokenCreationFailed
    })?;

    let mut token_claims: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    token_claims.insert("sub".to_string(), serde_json::json!(claims.sub));
    token_claims.insert("role".to_string(), serde_json::to_value(&claims.role).unwrap());
    token_claims.insert("exp".to_string(), serde_json::json!(claims.exp));
    token_claims.insert("iat".to_string(), serde_json::json!(claims.iat));
    token_claims.insert("iss".to_string(), serde_json::json!(claims.iss));

    token_claims.sign_with_key(&key).map_err(|e| {
        tracing::error!("Failed to encode JWT: {}", e);
        AuthError::TokenCreationFailed
    })
}

/// Decode and validate a JWT token.
pub fn decode_token(token: &str, secret: &str, issuer: &str) -> Result<Claims, AuthError> {
    let key = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| {
        tracing::error!("Failed to create HMAC key: {}", e);
        AuthError::InvalidToken
    })?;

    let token_claims: BTreeMap<String, serde_json::Value> =
        token.verify_with_key(&key).map_err(|e| {
            tracing::warn!("Failed to decode JWT: {}", e);
            AuthError::InvalidToken
        })?;

    // Extract claims
    let sub = token_claims
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or(AuthError::InvalidToken)?
        .to_string();

    let role: Role = token_claims
        .get("role")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .ok_or(AuthError::InvalidToken)?;

    let exp = token_claims
        .get("exp")
        .and_then(|v| v.as_i64())
        .ok_or(AuthError::InvalidToken)?;

    let iat = token_claims
        .get("iat")
        .and_then(|v| v.as_i64())
        .ok_or(AuthError::InvalidToken)?;

    let iss = token_claims
        .get("iss")
        .and_then(|v| v.as_str())
        .ok_or(AuthError::InvalidToken)?
        .to_string();

    // Validate issuer
    if iss != issuer {
        tracing::warn!("Invalid issuer: expected {}, got {}", issuer, iss);
        return Err(AuthError::InvalidToken);
    }

    let claims = Claims { sub, role, exp, iat, iss };

    if claims.is_expired() {
        return Err(AuthError::TokenExpired);
    }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_token() {
        let secret = "test-secret-key";
        let issuer = "test-issuer";
        let claims = Claims::new("user123", Role::User, issuer, 3600);

        let token = encode_token(&claims, secret).expect("Failed to encode");
        let decoded = decode_token(&token, secret, issuer).expect("Failed to decode");

        assert_eq!(decoded.sub, "user123");
        assert_eq!(decoded.role, Role::User);
        assert_eq!(decoded.iss, issuer);
    }
}
