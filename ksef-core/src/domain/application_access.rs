use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::user::UserId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApplicationAccessInviteId(Uuid);

impl ApplicationAccessInviteId {
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

impl Default for ApplicationAccessInviteId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ApplicationAccessInviteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ApplicationAccessInviteId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

#[derive(Debug, Clone)]
pub struct ApplicationAccessInvite {
    pub id: ApplicationAccessInviteId,
    pub email: String,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_by_user_id: UserId,
    pub created_at: DateTime<Utc>,
}
