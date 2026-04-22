use sqlx::PgExecutor;
use sqlx::types::Json;

use crate::domain::account_scope::AccountScope;
use crate::domain::nip_account::NipAccountId;
use crate::domain::token_mgmt::LocalToken;
use crate::domain::user::UserId;
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct LocalTokenRow {
    id: uuid::Uuid,
    nip_account_id: uuid::Uuid,
    user_id: uuid::Uuid,
    ksef_token_id: String,
    permissions: Json<Vec<crate::domain::permission::PermissionType>>,
    description: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl LocalTokenRow {
    fn into_domain(self) -> LocalToken {
        LocalToken {
            id: self.id,
            nip_account_id: NipAccountId::from_uuid(self.nip_account_id),
            user_id: UserId::from_uuid(self.user_id),
            ksef_token_id: self.ksef_token_id,
            permissions: self.permissions.0,
            description: self.description,
            created_at: self.created_at,
            revoked_at: self.revoked_at,
        }
    }
}

pub async fn save<'e>(
    exec: impl PgExecutor<'e>,
    token: &LocalToken,
) -> Result<(), RepositoryError> {
    sqlx::query(
        r"INSERT INTO nip_account_tokens
            (id, nip_account_id, user_id, ksef_token_id, permissions, description, created_at, revoked_at)
          VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
          ON CONFLICT (ksef_token_id) DO NOTHING",
    )
    .bind(token.id)
    .bind(token.nip_account_id.as_uuid())
    .bind(token.user_id.as_uuid())
    .bind(&token.ksef_token_id)
    .bind(Json(token.permissions.clone()))
    .bind(&token.description)
    .bind(token.created_at)
    .bind(token.revoked_at)
    .execute(exec)
    .await
    .map_err(RepositoryError::Database)?;

    Ok(())
}

pub async fn list_by_account<'e>(
    exec: impl PgExecutor<'e>,
    scope: &AccountScope,
) -> Result<Vec<LocalToken>, RepositoryError> {
    let rows: Vec<LocalTokenRow> = sqlx::query_as(
        r"SELECT id, nip_account_id, user_id, ksef_token_id, permissions, description, created_at, revoked_at
          FROM nip_account_tokens
          WHERE nip_account_id = $1
          ORDER BY created_at DESC",
    )
    .bind(scope.id().as_uuid())
    .fetch_all(exec)
    .await
    .map_err(RepositoryError::Database)?;

    Ok(rows.into_iter().map(LocalTokenRow::into_domain).collect())
}

pub async fn list_by_account_for_user<'e>(
    exec: impl PgExecutor<'e>,
    scope: &AccountScope,
    user_id: &UserId,
) -> Result<Vec<LocalToken>, RepositoryError> {
    let rows: Vec<LocalTokenRow> = sqlx::query_as(
        r"SELECT id, nip_account_id, user_id, ksef_token_id, permissions, description, created_at, revoked_at
          FROM nip_account_tokens
          WHERE nip_account_id = $1 AND user_id = $2
          ORDER BY created_at DESC",
    )
    .bind(scope.id().as_uuid())
    .bind(user_id.as_uuid())
    .fetch_all(exec)
    .await
    .map_err(RepositoryError::Database)?;

    Ok(rows.into_iter().map(LocalTokenRow::into_domain).collect())
}

pub async fn mark_revoked<'e>(
    exec: impl PgExecutor<'e>,
    ksef_token_id: &str,
    scope: &AccountScope,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "UPDATE nip_account_tokens SET revoked_at = COALESCE(revoked_at, NOW()) WHERE ksef_token_id = $1 AND nip_account_id = $2",
    )
    .bind(ksef_token_id)
    .bind(scope.id().as_uuid())
    .execute(exec)
    .await
    .map_err(RepositoryError::Database)?;

    Ok(())
}
