use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::nip::Nip;

/// Unique identifier for a NIP account.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NipAccountId(Uuid);

impl NipAccountId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    #[must_use]
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for NipAccountId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NipAccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for NipAccountId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// KSeF authentication method for this NIP account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KSeFAuthMethod {
    Xades,
    Token,
}

impl fmt::Display for KSeFAuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xades => f.write_str("xades"),
            Self::Token => f.write_str("token"),
        }
    }
}

impl FromStr for KSeFAuthMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "xades" => Ok(Self::Xades),
            "token" => Ok(Self::Token),
            other => Err(format!("invalid KSeF auth method: '{other}'")),
        }
    }
}

/// A NIP account with KSeF credentials, owned by one or more users.
#[derive(Debug, Clone)]
pub struct NipAccount {
    pub id: NipAccountId,
    pub nip: Nip,
    pub display_name: String,
    pub ksef_auth_method: KSeFAuthMethod,
    pub ksef_auth_token: Option<String>,
    pub cert_pem: Option<Vec<u8>>,
    pub key_pem: Option<Vec<u8>>,
    pub cert_auto_generated: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nip_account_id_round_trips_through_string() {
        let id = NipAccountId::new();
        let s = id.to_string();
        let parsed: NipAccountId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn nip_account_id_rejects_invalid_uuid() {
        let result = "not-a-uuid".parse::<NipAccountId>();
        assert!(result.is_err());
    }

    #[test]
    fn ksef_auth_method_round_trips() {
        assert_eq!(
            "xades".parse::<KSeFAuthMethod>().unwrap(),
            KSeFAuthMethod::Xades
        );
        assert_eq!(
            "token".parse::<KSeFAuthMethod>().unwrap(),
            KSeFAuthMethod::Token
        );
        assert_eq!(KSeFAuthMethod::Xades.to_string(), "xades");
        assert_eq!(KSeFAuthMethod::Token.to_string(), "token");
    }

    #[test]
    fn ksef_auth_method_rejects_invalid() {
        assert!("invalid".parse::<KSeFAuthMethod>().is_err());
    }

    #[test]
    fn ksef_auth_method_case_insensitive() {
        assert_eq!(
            "XADES".parse::<KSeFAuthMethod>().unwrap(),
            KSeFAuthMethod::Xades
        );
        assert_eq!(
            "Token".parse::<KSeFAuthMethod>().unwrap(),
            KSeFAuthMethod::Token
        );
    }
}
