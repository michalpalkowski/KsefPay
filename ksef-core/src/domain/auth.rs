use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::nip::Nip;

/// Challenge returned by `POST /auth/challenge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChallenge {
    pub timestamp: String,
    pub challenge: String,
}

/// Reference number returned after submitting auth request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthReference {
    reference_number: String,
    authentication_token: String,
}

impl AuthReference {
    #[must_use]
    pub fn new(reference_number: String, authentication_token: String) -> Self {
        Self {
            reference_number,
            authentication_token,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.reference_number
    }

    #[must_use]
    pub fn authentication_token(&self) -> &str {
        &self.authentication_token
    }
}

impl fmt::Display for AuthReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reference_number)
    }
}

/// Status of an ongoing auth request (from polling `GET /auth/{ref}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthStatus {
    Processing,
    Completed,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextIdentifier {
    Nip(Nip),
}

impl ContextIdentifier {
    #[must_use]
    pub fn api_type(&self) -> &'static str {
        match self {
            Self::Nip(_) => "onip",
        }
    }

    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::Nip(nip) => nip.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSessionInfo {
    pub reference_number: String,
    pub created_at: DateTime<Utc>,
    pub current: bool,
}

/// JWT access + refresh token pair from `POST /auth/token/redeem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: AccessToken,
    pub refresh_token: RefreshToken,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token_expires_at: DateTime<Utc>,
}

impl TokenPair {
    #[must_use]
    pub fn is_access_expired(&self) -> bool {
        Utc::now() >= self.access_token_expires_at
    }

    #[must_use]
    pub fn is_refresh_expired(&self) -> bool {
        Utc::now() >= self.refresh_token_expires_at
    }
}

/// Opaque JWT access token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken(String);

impl AccessToken {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque JWT refresh token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshToken(String);

impl RefreshToken {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_reference_encapsulated() {
        let r = AuthReference::new("ref-123".to_string(), "auth-token-xyz".to_string());
        assert_eq!(r.as_str(), "ref-123");
        assert_eq!(r.authentication_token(), "auth-token-xyz");
        assert_eq!(r.to_string(), "ref-123");
    }

    #[test]
    fn access_token_encapsulated() {
        let t = AccessToken::new("jwt.token.here".to_string());
        assert_eq!(t.as_str(), "jwt.token.here");
    }

    #[test]
    fn token_pair_expired_access_token() {
        let pair = TokenPair {
            access_token: AccessToken::new("test".to_string()),
            refresh_token: RefreshToken::new("test".to_string()),
            access_token_expires_at: Utc::now() - chrono::Duration::minutes(1),
            refresh_token_expires_at: Utc::now() + chrono::Duration::days(7),
        };
        assert!(pair.is_access_expired());
        assert!(!pair.is_refresh_expired());
    }

    #[test]
    fn token_pair_valid_access_token() {
        let pair = TokenPair {
            access_token: AccessToken::new("test".to_string()),
            refresh_token: RefreshToken::new("test".to_string()),
            access_token_expires_at: Utc::now() + chrono::Duration::minutes(15),
            refresh_token_expires_at: Utc::now() + chrono::Duration::days(7),
        };
        assert!(!pair.is_access_expired());
        assert!(!pair.is_refresh_expired());
    }

    #[test]
    fn context_identifier_nip_maps_to_api_shape() {
        let nip = Nip::parse("5260250274").unwrap();
        let context = ContextIdentifier::Nip(nip);
        assert_eq!(context.api_type(), "onip");
        assert_eq!(context.value(), "5260250274");
    }
}
