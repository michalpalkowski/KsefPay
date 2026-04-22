use sqlx::SqliteExecutor;

use super::datetime::parse_sqlite_datetime;
use crate::domain::application_access::{ApplicationAccessInvite, ApplicationAccessInviteId};
use crate::domain::user::UserId;
use crate::error::RepositoryError;

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

fn parse_datetime(
    value: &str,
    field: &'static str,
) -> Result<chrono::DateTime<chrono::Utc>, RepositoryError> {
    parse_sqlite_datetime(value, field).map_err(decode_err)
}

#[derive(sqlx::FromRow)]
struct ApplicationAccessInviteRow {
    id: String,
    email: String,
    token_hash: String,
    expires_at: String,
    accepted_at: Option<String>,
    revoked_at: Option<String>,
    created_by_user_id: String,
    created_at: String,
}

impl ApplicationAccessInviteRow {
    fn into_domain(self) -> Result<ApplicationAccessInvite, RepositoryError> {
        let id = uuid::Uuid::parse_str(&self.id).map_err(|e| {
            decode_err(format!(
                "invalid application access invite id '{}': {e}",
                self.id
            ))
        })?;
        let created_by = uuid::Uuid::parse_str(&self.created_by_user_id).map_err(|e| {
            decode_err(format!(
                "invalid application access invite created_by_user_id '{}': {e}",
                self.created_by_user_id
            ))
        })?;

        Ok(ApplicationAccessInvite {
            id: ApplicationAccessInviteId::from_uuid(id),
            email: self.email,
            token_hash: self.token_hash,
            expires_at: parse_datetime(&self.expires_at, "expires_at")?,
            accepted_at: self
                .accepted_at
                .as_deref()
                .map(|value| parse_datetime(value, "accepted_at"))
                .transpose()?,
            revoked_at: self
                .revoked_at
                .as_deref()
                .map(|value| parse_datetime(value, "revoked_at"))
                .transpose()?,
            created_by_user_id: UserId::from_uuid(created_by),
            created_at: parse_datetime(&self.created_at, "created_at")?,
        })
    }
}

pub async fn create_invite<'e, E>(
    exec: E,
    invite: &ApplicationAccessInvite,
) -> Result<ApplicationAccessInviteId, RepositoryError>
where
    E: SqliteExecutor<'e>,
{
    sqlx::query(
        r"INSERT INTO application_access_invites (
            id, email, token_hash, expires_at,
            accepted_at, revoked_at, created_by_user_id, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(invite.id.to_string())
    .bind(&invite.email)
    .bind(&invite.token_hash)
    .bind(invite.expires_at.to_rfc3339())
    .bind(invite.accepted_at.map(|value| value.to_rfc3339()))
    .bind(invite.revoked_at.map(|value| value.to_rfc3339()))
    .bind(invite.created_by_user_id.to_string())
    .bind(invite.created_at.to_rfc3339())
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "ApplicationAccessInvite",
                key: invite.token_hash.clone(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(invite.id.clone())
}

pub async fn list_pending_invites<'e, E>(
    exec: E,
) -> Result<Vec<ApplicationAccessInvite>, RepositoryError>
where
    E: SqliteExecutor<'e>,
{
    let rows: Vec<ApplicationAccessInviteRow> = sqlx::query_as(
        r"SELECT * FROM application_access_invites
          WHERE accepted_at IS NULL
            AND revoked_at IS NULL
            AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ','now')
          ORDER BY created_at DESC",
    )
    .fetch_all(exec)
    .await?;

    rows.into_iter()
        .map(ApplicationAccessInviteRow::into_domain)
        .collect()
}

pub async fn find_invite_by_token_hash<'e, E>(
    exec: E,
    token_hash: &str,
) -> Result<Option<ApplicationAccessInvite>, RepositoryError>
where
    E: SqliteExecutor<'e>,
{
    let row: Option<ApplicationAccessInviteRow> =
        sqlx::query_as("SELECT * FROM application_access_invites WHERE token_hash = ?1")
            .bind(token_hash)
            .fetch_optional(exec)
            .await?;

    row.map(ApplicationAccessInviteRow::into_domain).transpose()
}

pub async fn accept_invite<'e, E>(
    exec: E,
    invite_id: &ApplicationAccessInviteId,
) -> Result<(), RepositoryError>
where
    E: SqliteExecutor<'e>,
{
    let result = sqlx::query(
        "UPDATE application_access_invites
            SET accepted_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE id = ?1
            AND accepted_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(invite_id.to_string())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "ApplicationAccessInvite",
            id: invite_id.to_string(),
        });
    }

    Ok(())
}

pub async fn revoke_invite<'e, E>(
    exec: E,
    invite_id: &ApplicationAccessInviteId,
) -> Result<(), RepositoryError>
where
    E: SqliteExecutor<'e>,
{
    let result = sqlx::query(
        "UPDATE application_access_invites
            SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE id = ?1
            AND accepted_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(invite_id.to_string())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "ApplicationAccessInvite",
            id: invite_id.to_string(),
        });
    }

    Ok(())
}
