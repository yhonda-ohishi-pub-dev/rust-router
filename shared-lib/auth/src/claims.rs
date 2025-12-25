//! JWT claims and role definitions.

use serde::{Deserialize, Serialize};

/// User roles in the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Administrator with full access
    Admin,
    /// Regular user
    User,
    /// Read-only access
    Viewer,
}

impl Default for Role {
    fn default() -> Self {
        Self::User
    }
}

/// JWT claims structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID)
    pub sub: String,
    /// User's role
    pub role: Role,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Issuer
    pub iss: String,
}

impl Claims {
    /// Create new claims for a user.
    pub fn new(user_id: impl Into<String>, role: Role, issuer: impl Into<String>, expires_in_secs: i64) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            sub: user_id.into(),
            role,
            exp: now + expires_in_secs,
            iat: now,
            iss: issuer.into(),
        }
    }

    /// Check if the claims have expired.
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() > self.exp
    }

    /// Check if the user has admin role.
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}
