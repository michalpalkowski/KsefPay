use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::nip_account::NipAccountId;
use crate::domain::permission::PermissionType;
use crate::domain::user::UserId;
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

/// A token entry stored locally, scoped to a specific NIP account and user.
///
/// Unlike `ManagedToken` (which reflects the live KSeF API response), this
/// is persisted in the local DB so that the tokens page can be filtered per-NIP.
#[derive(Debug, Clone)]
pub struct LocalToken {
    pub id: Uuid,
    pub nip_account_id: NipAccountId,
    pub user_id: UserId,
    pub ksef_token_id: String,
    pub permissions: Vec<PermissionType>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl LocalToken {
    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
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
