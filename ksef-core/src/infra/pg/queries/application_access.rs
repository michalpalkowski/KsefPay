use sqlx::PgExecutor;

use crate::domain::application_access::{
    ApplicationAccessInvite, ApplicationAccessInviteId, TrustedApplicationEmailAccess,
    TrustedApplicationEmailAccessId,
};
use crate::domain::user::UserId;
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct ApplicationAccessInviteRow {
    id: uuid::Uuid,
    email: String,
    token_hash: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by_user_id: uuid::Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct TrustedApplicationEmailAccessRow {
    id: uuid::Uuid,
    email: String,
    consumed_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by_user_id: uuid::Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ApplicationAccessInviteRow {
    fn into_domain(self) -> Result<ApplicationAccessInvite, RepositoryError> {
        Ok(ApplicationAccessInvite {
            id: ApplicationAccessInviteId::from_uuid(self.id),
            email: self.email,
            token_hash: self.token_hash,
            expires_at: self.expires_at,
            accepted_at: self.accepted_at,
            revoked_at: self.revoked_at,
            created_by_user_id: UserId::from_uuid(self.created_by_user_id),
            created_at: self.created_at,
        })
    }
}

impl TrustedApplicationEmailAccessRow {
    fn into_domain(self) -> Result<TrustedApplicationEmailAccess, RepositoryError> {
        Ok(TrustedApplicationEmailAccess {
            id: TrustedApplicationEmailAccessId::from_uuid(self.id),
            email: self.email,
            consumed_at: self.consumed_at,
            revoked_at: self.revoked_at,
            created_by_user_id: UserId::from_uuid(self.created_by_user_id),
            created_at: self.created_at,
        })
    }
}

pub async fn create_invite<'e, E>(
    exec: E,
    invite: &ApplicationAccessInvite,
) -> Result<ApplicationAccessInviteId, RepositoryError>
where
    E: PgExecutor<'e>,
{
    sqlx::query(
        r"INSERT INTO application_access_invites (
            id, email, token_hash, expires_at,
            accepted_at, revoked_at, created_by_user_id, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(invite.id.as_uuid())
    .bind(&invite.email)
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
    E: PgExecutor<'e>,
{
    let rows: Vec<ApplicationAccessInviteRow> = sqlx::query_as(
        r"SELECT * FROM application_access_invites
          WHERE accepted_at IS NULL
            AND revoked_at IS NULL
            AND expires_at > NOW()
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
    E: PgExecutor<'e>,
{
    let row: Option<ApplicationAccessInviteRow> =
        sqlx::query_as("SELECT * FROM application_access_invites WHERE token_hash = $1")
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
    E: PgExecutor<'e>,
{
    let result = sqlx::query(
        "UPDATE application_access_invites
            SET accepted_at = NOW()
          WHERE id = $1
            AND accepted_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(invite_id.as_uuid())
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
    E: PgExecutor<'e>,
{
    let result = sqlx::query(
        "UPDATE application_access_invites
            SET revoked_at = NOW()
          WHERE id = $1
            AND accepted_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(invite_id.as_uuid())
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

pub async fn create_trusted_email_access<'e, E>(
    exec: E,
    access: &TrustedApplicationEmailAccess,
) -> Result<TrustedApplicationEmailAccessId, RepositoryError>
where
    E: PgExecutor<'e>,
{
    sqlx::query(
        r"INSERT INTO trusted_application_email_access (
            id, email, consumed_at, revoked_at, created_by_user_id, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(access.id.as_uuid())
    .bind(&access.email)
    .bind(access.consumed_at)
    .bind(access.revoked_at)
    .bind(access.created_by_user_id.as_uuid())
    .bind(access.created_at)
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "TrustedApplicationEmailAccess",
                key: access.email.clone(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(access.id.clone())
}

pub async fn list_pending_trusted_email_access<'e, E>(
    exec: E,
) -> Result<Vec<TrustedApplicationEmailAccess>, RepositoryError>
where
    E: PgExecutor<'e>,
{
    let rows: Vec<TrustedApplicationEmailAccessRow> = sqlx::query_as(
        r"SELECT * FROM trusted_application_email_access
          WHERE consumed_at IS NULL
            AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .fetch_all(exec)
    .await?;

    rows.into_iter()
        .map(TrustedApplicationEmailAccessRow::into_domain)
        .collect()
}

pub async fn find_pending_trusted_email_access_by_email<'e, E>(
    exec: E,
    email: &str,
) -> Result<Option<TrustedApplicationEmailAccess>, RepositoryError>
where
    E: PgExecutor<'e>,
{
    let row: Option<TrustedApplicationEmailAccessRow> = sqlx::query_as(
        r"SELECT * FROM trusted_application_email_access
          WHERE email = $1
            AND consumed_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(email)
    .fetch_optional(exec)
    .await?;

    row.map(TrustedApplicationEmailAccessRow::into_domain)
        .transpose()
}

pub async fn consume_trusted_email_access<'e, E>(
    exec: E,
    access_id: &TrustedApplicationEmailAccessId,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e>,
{
    let result = sqlx::query(
        "UPDATE trusted_application_email_access
            SET consumed_at = NOW()
          WHERE id = $1
            AND consumed_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(access_id.as_uuid())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "TrustedApplicationEmailAccess",
            id: access_id.to_string(),
        });
    }

    Ok(())
}

pub async fn revoke_trusted_email_access<'e, E>(
    exec: E,
    access_id: &TrustedApplicationEmailAccessId,
) -> Result<(), RepositoryError>
where
    E: PgExecutor<'e>,
{
    let result = sqlx::query(
        "UPDATE trusted_application_email_access
            SET revoked_at = NOW()
          WHERE id = $1
            AND consumed_at IS NULL
            AND revoked_at IS NULL",
    )
    .bind(access_id.as_uuid())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "TrustedApplicationEmailAccess",
            id: access_id.to_string(),
        });
    }

    Ok(())
}
