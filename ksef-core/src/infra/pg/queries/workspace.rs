use chrono::Utc;
use sqlx::PgExecutor;

use super::nip_account::NipAccountRow;
use crate::domain::account_scope::AccountScope;
use crate::domain::nip::Nip;
use crate::domain::nip_account::{NipAccount, NipAccountId};
use crate::domain::user::UserId;
use crate::domain::workspace::{
    Workspace, WorkspaceId, WorkspaceInvite, WorkspaceInviteId, WorkspaceMembership,
    WorkspaceMembershipStatus, WorkspaceNipOwnership, WorkspaceRole, WorkspaceSummary,
};
use crate::error::RepositoryError;
use crate::infra::crypto::CertificateSecretBox;

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

#[derive(sqlx::FromRow)]
struct WorkspaceRow {
    id: uuid::Uuid,
    slug: String,
    display_name: String,
    created_by_user_id: uuid::Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl WorkspaceRow {
    fn into_domain(self) -> Workspace {
        Workspace {
            id: WorkspaceId::from_uuid(self.id),
            slug: self.slug,
            display_name: self.display_name,
            created_by_user_id: UserId::from_uuid(self.created_by_user_id),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceMembershipRow {
    workspace_id: uuid::Uuid,
    user_id: uuid::Uuid,
    role: String,
    status: String,
    can_manage_members: bool,
    can_manage_nips: bool,
    can_manage_credentials: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl WorkspaceMembershipRow {
    fn into_domain(self) -> Result<WorkspaceMembership, RepositoryError> {
        Ok(WorkspaceMembership {
            workspace_id: WorkspaceId::from_uuid(self.workspace_id),
            user_id: UserId::from_uuid(self.user_id),
            role: self.role.parse().map_err(decode_err)?,
            status: self.status.parse().map_err(decode_err)?,
            can_manage_members: self.can_manage_members,
            can_manage_nips: self.can_manage_nips,
            can_manage_credentials: self.can_manage_credentials,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceSummaryRow {
    workspace_id: uuid::Uuid,
    workspace_slug: String,
    workspace_display_name: String,
    workspace_created_by_user_id: uuid::Uuid,
    workspace_created_at: chrono::DateTime<chrono::Utc>,
    workspace_updated_at: chrono::DateTime<chrono::Utc>,
    membership_user_id: uuid::Uuid,
    membership_role: String,
    membership_status: String,
    membership_can_manage_members: bool,
    membership_can_manage_nips: bool,
    membership_can_manage_credentials: bool,
    membership_created_at: chrono::DateTime<chrono::Utc>,
    membership_updated_at: chrono::DateTime<chrono::Utc>,
}

impl WorkspaceSummaryRow {
    fn into_domain(self) -> Result<WorkspaceSummary, RepositoryError> {
        Ok(WorkspaceSummary {
            workspace: Workspace {
                id: WorkspaceId::from_uuid(self.workspace_id),
                slug: self.workspace_slug,
                display_name: self.workspace_display_name,
                created_by_user_id: UserId::from_uuid(self.workspace_created_by_user_id),
                created_at: self.workspace_created_at,
                updated_at: self.workspace_updated_at,
            },
            membership: WorkspaceMembership {
                workspace_id: WorkspaceId::from_uuid(self.workspace_id),
                user_id: UserId::from_uuid(self.membership_user_id),
                role: self.membership_role.parse().map_err(decode_err)?,
                status: self.membership_status.parse().map_err(decode_err)?,
                can_manage_members: self.membership_can_manage_members,
                can_manage_nips: self.membership_can_manage_nips,
                can_manage_credentials: self.membership_can_manage_credentials,
                created_at: self.membership_created_at,
                updated_at: self.membership_updated_at,
            },
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceInviteRow {
    id: uuid::Uuid,
    workspace_id: uuid::Uuid,
    email: String,
    role: String,
    token_hash: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by_user_id: uuid::Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl WorkspaceInviteRow {
    fn into_domain(self) -> Result<WorkspaceInvite, RepositoryError> {
        Ok(WorkspaceInvite {
            id: WorkspaceInviteId::from_uuid(self.id),
            workspace_id: WorkspaceId::from_uuid(self.workspace_id),
            email: self.email,
            role: self.role.parse().map_err(decode_err)?,
            token_hash: self.token_hash,
            expires_at: self.expires_at,
            accepted_at: self.accepted_at,
            revoked_at: self.revoked_at,
            created_by_user_id: UserId::from_uuid(self.created_by_user_id),
            created_at: self.created_at,
        })
    }
}

fn slugify_personal_workspace(user_email: &str, user_id: &UserId) -> String {
    let local = user_email
        .split('@')
        .next()
        .unwrap_or("workspace")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = local.trim_matches('-');
    let prefix = if normalized.is_empty() {
        "workspace"
    } else {
        normalized
    };
    let user_id_str = user_id.to_string();
    format!("{prefix}-{}", &user_id_str[..8])
}

fn personal_workspace_name(user_email: &str) -> String {
    let local = user_email.split('@').next().unwrap_or("Workspace").trim();
    if local.is_empty() {
        "Workspace".to_string()
    } else {
        format!("{} workspace", local)
    }
}

fn merge_role(existing: WorkspaceRole, desired: WorkspaceRole) -> WorkspaceRole {
    use WorkspaceRole::{Admin, Operator, Owner, ReadOnly};

    match (existing, desired) {
        (Owner, _) | (_, Owner) => Owner,
        (Admin, _) | (_, Admin) => Admin,
        (Operator, _) | (_, Operator) => Operator,
        _ => ReadOnly,
    }
}

async fn first_workspace_for_user<'e, E>(
    exec: E,
    user_id: &UserId,
) -> Result<Option<WorkspaceSummary>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let row: Option<WorkspaceSummaryRow> = sqlx::query_as(
        r"SELECT
            w.id AS workspace_id,
            w.slug AS workspace_slug,
            w.display_name AS workspace_display_name,
            w.created_by_user_id AS workspace_created_by_user_id,
            w.created_at AS workspace_created_at,
            w.updated_at AS workspace_updated_at,
            wm.user_id AS membership_user_id,
            wm.role AS membership_role,
            wm.status AS membership_status,
            wm.can_manage_members AS membership_can_manage_members,
            wm.can_manage_nips AS membership_can_manage_nips,
            wm.can_manage_credentials AS membership_can_manage_credentials,
            wm.created_at AS membership_created_at,
            wm.updated_at AS membership_updated_at
        FROM workspace_memberships wm
        INNER JOIN workspaces w ON w.id = wm.workspace_id
        WHERE wm.user_id = $1 AND wm.status = 'active'
        ORDER BY w.created_at, w.display_name
        LIMIT 1",
    )
    .bind(user_id.as_uuid())
    .fetch_optional(exec)
    .await?;

    row.map(WorkspaceSummaryRow::into_domain).transpose()
}

async fn ensure_workspace_for_user<'e, E>(
    exec: E,
    user_id: &UserId,
    user_email: &str,
) -> Result<WorkspaceSummary, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    if let Some(summary) = first_workspace_for_user(exec, user_id).await? {
        return Ok(summary);
    }

    let now = Utc::now();
    let workspace = Workspace {
        id: WorkspaceId::new(),
        slug: slugify_personal_workspace(user_email, user_id),
        display_name: personal_workspace_name(user_email),
        created_by_user_id: user_id.clone(),
        created_at: now,
        updated_at: now,
    };
    create_workspace(exec, &workspace, user_id).await?;
    first_workspace_for_user(exec, user_id)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "Workspace",
            id: workspace.id.to_string(),
        })
}

async fn upsert_membership<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
    user_id: &UserId,
    role: WorkspaceRole,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let now = Utc::now();
    let existing = find_membership(exec, workspace_id, user_id).await?;
    let role = existing
        .map(|membership| merge_role(membership.role, role))
        .unwrap_or(role);

    sqlx::query(
        r"INSERT INTO workspace_memberships (
            workspace_id, user_id, role, status,
            can_manage_members, can_manage_nips, can_manage_credentials,
            created_at, updated_at
        ) VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8)
        ON CONFLICT (workspace_id, user_id) DO UPDATE SET
            role = EXCLUDED.role,
            status = 'active',
            can_manage_members = EXCLUDED.can_manage_members,
            can_manage_nips = EXCLUDED.can_manage_nips,
            can_manage_credentials = EXCLUDED.can_manage_credentials,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(workspace_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(role.to_string())
    .bind(role.can_manage_members())
    .bind(role.can_manage_nips())
    .bind(role.can_manage_credentials())
    .bind(now)
    .bind(now)
    .execute(exec)
    .await?;

    Ok(())
}

pub async fn create_workspace<'e, E>(
    exec: E,
    workspace: &Workspace,
    owner_id: &UserId,
) -> Result<WorkspaceId, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    sqlx::query(
        r"INSERT INTO workspaces (id, slug, display_name, created_by_user_id, created_at, updated_at)
          VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(workspace.id.as_uuid())
    .bind(&workspace.slug)
    .bind(&workspace.display_name)
    .bind(workspace.created_by_user_id.as_uuid())
    .bind(workspace.created_at)
    .bind(workspace.updated_at)
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "Workspace",
                key: workspace.slug.clone(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    upsert_membership(exec, &workspace.id, owner_id, WorkspaceRole::Owner).await?;
    Ok(workspace.id.clone())
}

pub async fn ensure_default_workspace<'e, E>(
    exec: E,
    user_id: &UserId,
    user_email: &str,
) -> Result<WorkspaceSummary, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    ensure_workspace_for_user(exec, user_id, user_email).await
}

pub async fn find_by_id<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
) -> Result<Workspace, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let row: WorkspaceRow = sqlx::query_as("SELECT * FROM workspaces WHERE id = $1")
        .bind(workspace_id.as_uuid())
        .fetch_optional(exec)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "Workspace",
            id: workspace_id.to_string(),
        })?;

    Ok(row.into_domain())
}

pub async fn list_for_user<'e, E>(
    exec: E,
    user_id: &UserId,
) -> Result<Vec<WorkspaceSummary>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let rows: Vec<WorkspaceSummaryRow> = sqlx::query_as(
        r"SELECT
            w.id AS workspace_id,
            w.slug AS workspace_slug,
            w.display_name AS workspace_display_name,
            w.created_by_user_id AS workspace_created_by_user_id,
            w.created_at AS workspace_created_at,
            w.updated_at AS workspace_updated_at,
            wm.user_id AS membership_user_id,
            wm.role AS membership_role,
            wm.status AS membership_status,
            wm.can_manage_members AS membership_can_manage_members,
            wm.can_manage_nips AS membership_can_manage_nips,
            wm.can_manage_credentials AS membership_can_manage_credentials,
            wm.created_at AS membership_created_at,
            wm.updated_at AS membership_updated_at
        FROM workspace_memberships wm
        INNER JOIN workspaces w ON w.id = wm.workspace_id
        WHERE wm.user_id = $1 AND wm.status = 'active'
        ORDER BY w.display_name, w.created_at",
    )
    .bind(user_id.as_uuid())
    .fetch_all(exec)
    .await?;

    rows.into_iter()
        .map(WorkspaceSummaryRow::into_domain)
        .collect()
}

pub async fn find_membership<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
    user_id: &UserId,
) -> Result<Option<WorkspaceMembership>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let row: Option<WorkspaceMembershipRow> = sqlx::query_as(
        r"SELECT * FROM workspace_memberships
          WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id.as_uuid())
    .bind(user_id.as_uuid())
    .fetch_optional(exec)
    .await?;

    row.map(WorkspaceMembershipRow::into_domain).transpose()
}

pub async fn add_member<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
    user_id: &UserId,
    role: WorkspaceRole,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    upsert_membership(exec, workspace_id, user_id, role).await
}

pub async fn attach_nip<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
    account_id: &NipAccountId,
    ownership: WorkspaceNipOwnership,
    attached_by: &UserId,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    sqlx::query(
        r"INSERT INTO workspace_nip_accounts (
            workspace_id, nip_account_id, ownership_type, attached_by_user_id, created_at
        ) VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (workspace_id, nip_account_id) DO UPDATE
          SET ownership_type = EXCLUDED.ownership_type,
              attached_by_user_id = EXCLUDED.attached_by_user_id",
    )
    .bind(workspace_id.as_uuid())
    .bind(account_id.as_uuid())
    .bind(ownership.to_string())
    .bind(attached_by.as_uuid())
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "WorkspaceNipAccount",
                key: account_id.to_string(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(())
}

pub async fn list_nip_accounts_for_user<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
    user_id: &UserId,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<Vec<NipAccount>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let rows: Vec<NipAccountRow> = sqlx::query_as(
        r"SELECT DISTINCT na.*
          FROM nip_accounts na
          INNER JOIN workspace_nip_accounts wna ON wna.nip_account_id = na.id
          INNER JOIN workspace_memberships wm ON wm.workspace_id = wna.workspace_id
          WHERE wna.workspace_id = $1
            AND wm.user_id = $2
            AND wm.status = 'active'
          ORDER BY na.display_name, na.nip",
    )
    .bind(workspace_id.as_uuid())
    .bind(user_id.as_uuid())
    .fetch_all(exec)
    .await?;

    rows.into_iter()
        .map(|row| row.into_domain(certificate_secret_box))
        .collect()
}

pub async fn find_user_account_in_workspace<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
    user_id: &UserId,
    nip: &Nip,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<Option<(NipAccount, AccountScope, WorkspaceMembership)>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let membership = match find_membership(exec, workspace_id, user_id).await? {
        Some(membership) if membership.status == WorkspaceMembershipStatus::Active => membership,
        _ => return Ok(None),
    };

    let row: Option<NipAccountRow> = sqlx::query_as(
        r"SELECT na.*
          FROM nip_accounts na
          INNER JOIN workspace_nip_accounts wna ON wna.nip_account_id = na.id
          WHERE wna.workspace_id = $1 AND na.nip = $2",
    )
    .bind(workspace_id.as_uuid())
    .bind(nip.as_str())
    .fetch_optional(exec)
    .await?;

    row.map(|record| {
        let account = record.into_domain(certificate_secret_box)?;
        let scope = AccountScope::new(account.id.clone(), account.nip.clone());
        Ok((account, scope, membership))
    })
    .transpose()
}

pub async fn create_invite<'e, E>(
    exec: E,
    invite: &WorkspaceInvite,
) -> Result<WorkspaceInviteId, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    sqlx::query(
        r"INSERT INTO workspace_invites (
            id, workspace_id, email, role, token_hash, expires_at,
            accepted_at, revoked_at, created_by_user_id, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(invite.id.as_uuid())
    .bind(invite.workspace_id.as_uuid())
    .bind(&invite.email)
    .bind(invite.role.to_string())
    .bind(&invite.token_hash)
    .bind(invite.expires_at)
    .bind(invite.accepted_at)
    .bind(invite.revoked_at)
    .bind(invite.created_by_user_id.as_uuid())
    .bind(invite.created_at)
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "WorkspaceInvite",
                key: invite.token_hash.clone(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(invite.id.clone())
}

pub async fn list_pending_invites<'e, E>(
    exec: E,
    workspace_id: &WorkspaceId,
) -> Result<Vec<WorkspaceInvite>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let rows: Vec<WorkspaceInviteRow> = sqlx::query_as(
        r"SELECT * FROM workspace_invites
          WHERE workspace_id = $1
            AND accepted_at IS NULL
            AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(workspace_id.as_uuid())
    .fetch_all(exec)
    .await?;

    rows.into_iter()
        .map(WorkspaceInviteRow::into_domain)
        .collect()
}

pub async fn find_invite_by_token_hash<'e, E>(
    exec: E,
    token_hash: &str,
) -> Result<Option<WorkspaceInvite>, RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    let row: Option<WorkspaceInviteRow> =
        sqlx::query_as("SELECT * FROM workspace_invites WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(exec)
            .await?;

    row.map(WorkspaceInviteRow::into_domain).transpose()
}

pub async fn accept_invite<'e, E>(
    exec: E,
    invite_id: &WorkspaceInviteId,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    sqlx::query(
        "UPDATE workspace_invites SET accepted_at = NOW() WHERE id = $1 AND accepted_at IS NULL",
    )
    .bind(invite_id.as_uuid())
    .execute(exec)
    .await?;
    Ok(())
}

pub async fn revoke_invite<'e, E>(
    exec: E,
    invite_id: &WorkspaceInviteId,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e> + Copy,
{
    sqlx::query(
        "UPDATE workspace_invites SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(invite_id.as_uuid())
    .execute(exec)
    .await?;
    Ok(())
}
