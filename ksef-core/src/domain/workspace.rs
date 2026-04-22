use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::user::UserId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(Uuid);

impl WorkspaceId {
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

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for WorkspaceId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceInviteId(Uuid);

impl WorkspaceInviteId {
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

impl Default for WorkspaceInviteId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WorkspaceInviteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for WorkspaceInviteId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Operator,
    ReadOnly,
}

impl WorkspaceRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::ReadOnly => "read_only",
        }
    }

    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Owner => "Owner",
            Self::Admin => "Admin",
            Self::Operator => "Operator",
            Self::ReadOnly => "Read only",
        }
    }

    #[must_use]
    pub fn can_manage_members(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    #[must_use]
    pub fn can_manage_nips(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    #[must_use]
    pub fn can_manage_credentials(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl fmt::Display for WorkspaceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkspaceRole {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "operator" => Ok(Self::Operator),
            "read_only" => Ok(Self::ReadOnly),
            other => Err(format!("invalid workspace role: '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceMembershipStatus {
    Active,
    Invited,
    Revoked,
}

impl WorkspaceMembershipStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Invited => "invited",
            Self::Revoked => "revoked",
        }
    }
}

impl fmt::Display for WorkspaceMembershipStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkspaceMembershipStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "invited" => Ok(Self::Invited),
            "revoked" => Ok(Self::Revoked),
            other => Err(format!("invalid workspace membership status: '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceNipOwnership {
    WorkspaceOwned,
    Delegated,
    MigratedLegacy,
}

impl WorkspaceNipOwnership {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceOwned => "workspace_owned",
            Self::Delegated => "delegated",
            Self::MigratedLegacy => "migrated_legacy",
        }
    }
}

impl fmt::Display for WorkspaceNipOwnership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkspaceNipOwnership {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "workspace_owned" => Ok(Self::WorkspaceOwned),
            "delegated" => Ok(Self::Delegated),
            "migrated_legacy" => Ok(Self::MigratedLegacy),
            other => Err(format!("invalid workspace NIP ownership: '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub slug: String,
    pub display_name: String,
    pub created_by_user_id: UserId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceMembership {
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub role: WorkspaceRole,
    pub status: WorkspaceMembershipStatus,
    pub can_manage_members: bool,
    pub can_manage_nips: bool,
    pub can_manage_credentials: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceMembership {
    #[must_use]
    pub fn from_role(
        workspace_id: WorkspaceId,
        user_id: UserId,
        role: WorkspaceRole,
        status: WorkspaceMembershipStatus,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            workspace_id,
            user_id,
            role,
            status,
            can_manage_members: role.can_manage_members(),
            can_manage_nips: role.can_manage_nips(),
            can_manage_credentials: role.can_manage_credentials(),
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceSummary {
    pub workspace: Workspace,
    pub membership: WorkspaceMembership,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInvite {
    pub id: WorkspaceInviteId,
    pub workspace_id: WorkspaceId,
    pub email: String,
    pub role: WorkspaceRole,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_by_user_id: UserId,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_role_permissions_match_defaults() {
        assert!(WorkspaceRole::Owner.can_manage_members());
        assert!(WorkspaceRole::Owner.can_manage_nips());
        assert!(WorkspaceRole::Owner.can_manage_credentials());

        assert!(WorkspaceRole::Admin.can_manage_members());
        assert!(WorkspaceRole::Admin.can_manage_nips());
        assert!(WorkspaceRole::Admin.can_manage_credentials());

        assert!(!WorkspaceRole::Operator.can_manage_members());
        assert!(!WorkspaceRole::Operator.can_manage_nips());
        assert!(!WorkspaceRole::Operator.can_manage_credentials());

        assert!(!WorkspaceRole::ReadOnly.can_manage_members());
        assert!(!WorkspaceRole::ReadOnly.can_manage_nips());
        assert!(!WorkspaceRole::ReadOnly.can_manage_credentials());
    }

    #[test]
    fn workspace_role_round_trips() {
        let parsed: WorkspaceRole = "read_only".parse().unwrap();
        assert_eq!(parsed, WorkspaceRole::ReadOnly);
        assert_eq!(WorkspaceRole::Admin.to_string(), "admin");
    }
}
