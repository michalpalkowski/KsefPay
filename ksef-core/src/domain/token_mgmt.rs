use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::permission::PermissionType;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenStatus {
    Active,
    Revoked,
    Expired,
}

impl TokenStatus {
    pub fn transition_to(self, target: Self) -> Result<Self, DomainError> {
        let valid = matches!(
            (self, target),
            (Self::Active, Self::Revoked | Self::Expired)
        );

        if valid {
            Ok(target)
        } else {
            Err(DomainError::InvalidStatusTransition {
                from: self.to_string(),
                to: target.to_string(),
            })
        }
    }
}

impl fmt::Display for TokenStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Revoked => write!(f, "revoked"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedToken {
    pub id: String,
    pub status: TokenStatus,
    pub permissions: Vec<PermissionType>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_status_active_to_revoked_is_valid() {
        assert_eq!(
            TokenStatus::Active
                .transition_to(TokenStatus::Revoked)
                .unwrap(),
            TokenStatus::Revoked
        );
    }

    #[test]
    fn token_status_active_to_expired_is_valid() {
        assert_eq!(
            TokenStatus::Active
                .transition_to(TokenStatus::Expired)
                .unwrap(),
            TokenStatus::Expired
        );
    }

    #[test]
    fn token_status_terminal_transitions_are_invalid() {
        assert!(matches!(
            TokenStatus::Revoked.transition_to(TokenStatus::Active),
            Err(DomainError::InvalidStatusTransition { .. })
        ));
        assert!(matches!(
            TokenStatus::Expired.transition_to(TokenStatus::Active),
            Err(DomainError::InvalidStatusTransition { .. })
        ));
    }
}
