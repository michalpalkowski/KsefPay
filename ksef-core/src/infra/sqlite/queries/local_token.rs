use chrono::{DateTime, Utc};
use sqlx::SqliteExecutor;

use crate::domain::account_scope::AccountScope;
use crate::domain::nip_account::NipAccountId;
use crate::domain::permission::PermissionType;
use crate::domain::token_mgmt::LocalToken;
use crate::domain::user::UserId;
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct LocalTokenRow {
    id: String,
    nip_account_id: String,
    user_id: String,
    ksef_token_id: String,
    permissions: String,
    description: Option<String>,
    created_at: String,
    revoked_at: Option<String>,
}

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

impl LocalTokenRow {
    fn into_domain(self) -> Result<LocalToken, RepositoryError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| decode_err(format!("invalid local token id '{}': {e}", self.id)))?;

        let nip_account_id = self.nip_account_id.parse::<NipAccountId>().map_err(|e| {
            decode_err(format!(
                "invalid nip_account_id '{}': {e}",
                self.nip_account_id
            ))
        })?;

        let user_id = self
            .user_id
            .parse::<UserId>()
            .map_err(|e| decode_err(format!("invalid user_id '{}': {e}", self.user_id)))?;

        let permissions: Vec<PermissionType> = serde_json::from_str(&self.permissions)
            .map_err(|e| decode_err(format!("invalid permissions JSON: {e}")))?;

        let created_at: DateTime<Utc> = self
            .created_at
            .parse()
            .map_err(|e| decode_err(format!("invalid created_at '{}': {e}", self.created_at)))?;

        let revoked_at: Option<DateTime<Utc>> = self
            .revoked_at
            .as_deref()
            .map(|s| s.parse::<DateTime<Utc>>())
            .transpose()
            .map_err(|e| decode_err(format!("invalid revoked_at: {e}")))?;

        Ok(LocalToken {
            id,
            nip_account_id,
            user_id,
            ksef_token_id: self.ksef_token_id,
            permissions,
            description: self.description,
            created_at,
            revoked_at,
        })
    }
}

pub async fn save<'e>(
    exec: impl SqliteExecutor<'e>,
    token: &LocalToken,
) -> Result<(), RepositoryError> {
    let permissions_json = serde_json::to_string(&token.permissions)
        .map_err(|e| decode_err(format!("failed to serialize permissions: {e}")))?;

    sqlx::query(
        r"INSERT INTO nip_account_tokens
            (id, nip_account_id, user_id, ksef_token_id, permissions, description, created_at, revoked_at)
          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
          ON CONFLICT (ksef_token_id) DO NOTHING",
    )
    .bind(token.id.to_string())
    .bind(token.nip_account_id.to_string())
    .bind(token.user_id.to_string())
    .bind(&token.ksef_token_id)
    .bind(permissions_json)
    .bind(&token.description)
    .bind(token.created_at.to_rfc3339())
    .bind(token.revoked_at.map(|dt| dt.to_rfc3339()))
    .execute(exec)
    .await
    .map_err(RepositoryError::Database)?;

    Ok(())
}

pub async fn list_by_account<'e>(
    exec: impl SqliteExecutor<'e>,
    scope: &AccountScope,
) -> Result<Vec<LocalToken>, RepositoryError> {
    let rows: Vec<LocalTokenRow> = sqlx::query_as(
        "SELECT * FROM nip_account_tokens WHERE nip_account_id = ?1 ORDER BY datetime(created_at) DESC",
    )
    .bind(scope.id().to_string())
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(LocalTokenRow::into_domain).collect()
}

pub async fn list_by_account_for_user<'e>(
    exec: impl SqliteExecutor<'e>,
    scope: &AccountScope,
    user_id: &UserId,
) -> Result<Vec<LocalToken>, RepositoryError> {
    let rows: Vec<LocalTokenRow> = sqlx::query_as(
        r"SELECT *
          FROM nip_account_tokens
          WHERE nip_account_id = ?1 AND user_id = ?2
          ORDER BY datetime(created_at) DESC",
    )
    .bind(scope.id().to_string())
    .bind(user_id.to_string())
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(LocalTokenRow::into_domain).collect()
}

pub async fn mark_revoked<'e>(
    exec: impl SqliteExecutor<'e>,
    ksef_token_id: &str,
    scope: &AccountScope,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "UPDATE nip_account_tokens SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE ksef_token_id = ?1 AND nip_account_id = ?2",
    )
    .bind(ksef_token_id)
    .bind(scope.id().to_string())
    .execute(exec)
    .await
    .map_err(RepositoryError::Database)?;

    Ok(())
}
