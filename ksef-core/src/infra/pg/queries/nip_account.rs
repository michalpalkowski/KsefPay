use sqlx::PgExecutor;

use crate::domain::nip::Nip;
use crate::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
use crate::domain::user::UserId;
use crate::error::RepositoryError;

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

#[derive(sqlx::FromRow)]
struct NipAccountRow {
    id: uuid::Uuid,
    nip: String,
    display_name: String,
    ksef_auth_method: String,
    ksef_auth_token: Option<String>,
    cert_pem: Option<String>,
    key_pem: Option<String>,
    cert_auto_generated: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl NipAccountRow {
    fn into_domain(self) -> Result<NipAccount, RepositoryError> {
        let nip = Nip::parse(&self.nip)
            .map_err(|e| decode_err(format!("invalid NIP in nip_accounts: {e}")))?;
        let ksef_auth_method: KSeFAuthMethod = self
            .ksef_auth_method
            .parse()
            .map_err(|e: String| decode_err(e))?;

        Ok(NipAccount {
            id: NipAccountId::from_uuid(self.id),
            nip,
            display_name: self.display_name,
            ksef_auth_method,
            ksef_auth_token: self.ksef_auth_token,
            cert_pem: self.cert_pem.map(|s| s.into_bytes()),
            key_pem: self.key_pem.map(|s| s.into_bytes()),
            cert_auto_generated: self.cert_auto_generated,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

pub async fn create<'e>(
    exec: impl PgExecutor<'e>,
    account: &NipAccount,
) -> Result<NipAccountId, RepositoryError> {
    let cert_pem_str = account
        .cert_pem
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).into_owned());
    let key_pem_str = account
        .key_pem
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).into_owned());

    sqlx::query(
        r"INSERT INTO nip_accounts (
            id, nip, display_name, ksef_auth_method, ksef_auth_token,
            cert_pem, key_pem, cert_auto_generated, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(account.id.as_uuid())
    .bind(account.nip.as_str())
    .bind(&account.display_name)
    .bind(account.ksef_auth_method.to_string())
    .bind(&account.ksef_auth_token)
    .bind(&cert_pem_str)
    .bind(&key_pem_str)
    .bind(account.cert_auto_generated)
    .bind(account.created_at)
    .bind(account.updated_at)
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "NipAccount",
                key: account.nip.as_str().to_string(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(account.id.clone())
}

pub async fn find_by_id<'e>(
    exec: impl PgExecutor<'e>,
    id: &NipAccountId,
) -> Result<NipAccount, RepositoryError> {
    let row: NipAccountRow = sqlx::query_as("SELECT * FROM nip_accounts WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_optional(exec)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "NipAccount",
            id: id.to_string(),
        })?;
    row.into_domain()
}

pub async fn find_by_nip<'e>(
    exec: impl PgExecutor<'e>,
    nip: &Nip,
) -> Result<Option<NipAccount>, RepositoryError> {
    let row: Option<NipAccountRow> =
        sqlx::query_as("SELECT * FROM nip_accounts WHERE nip = $1")
            .bind(nip.as_str())
            .fetch_optional(exec)
            .await?;
    row.map(NipAccountRow::into_domain).transpose()
}

pub async fn update_credentials<'e>(
    exec: impl PgExecutor<'e>,
    account: &NipAccount,
) -> Result<(), RepositoryError> {
    let cert_pem_str = account
        .cert_pem
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).into_owned());
    let key_pem_str = account
        .key_pem
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).into_owned());

    let result = sqlx::query(
        r"UPDATE nip_accounts
        SET ksef_auth_method = $1,
            ksef_auth_token = $2,
            cert_pem = $3,
            key_pem = $4,
            cert_auto_generated = $5,
            updated_at = NOW()
        WHERE id = $6",
    )
    .bind(account.ksef_auth_method.to_string())
    .bind(&account.ksef_auth_token)
    .bind(&cert_pem_str)
    .bind(&key_pem_str)
    .bind(account.cert_auto_generated)
    .bind(account.id.as_uuid())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "NipAccount",
            id: account.id.to_string(),
        });
    }
    Ok(())
}

pub async fn grant_access<'e>(
    exec: impl PgExecutor<'e>,
    user_id: &UserId,
    account_id: &NipAccountId,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "INSERT INTO user_nip_access (user_id, nip_account_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user_id.as_uuid())
    .bind(account_id.as_uuid())
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "UserNipAccess",
                key: format!("{}:{}", user_id, account_id),
            }
        }
        _ => RepositoryError::Database(e),
    })?;
    Ok(())
}

pub async fn revoke_access<'e>(
    exec: impl PgExecutor<'e>,
    user_id: &UserId,
    account_id: &NipAccountId,
) -> Result<(), RepositoryError> {
    sqlx::query("DELETE FROM user_nip_access WHERE user_id = $1 AND nip_account_id = $2")
        .bind(user_id.as_uuid())
        .bind(account_id.as_uuid())
        .execute(exec)
        .await?;
    Ok(())
}

pub async fn list_by_user<'e>(
    exec: impl PgExecutor<'e>,
    user_id: &UserId,
) -> Result<Vec<NipAccount>, RepositoryError> {
    let rows: Vec<NipAccountRow> = sqlx::query_as(
        r"SELECT na.*
        FROM nip_accounts na
        INNER JOIN user_nip_access una ON una.nip_account_id = na.id
        WHERE una.user_id = $1
        ORDER BY na.created_at DESC",
    )
    .bind(user_id.as_uuid())
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(NipAccountRow::into_domain).collect()
}

pub async fn has_access<'e>(
    exec: impl PgExecutor<'e>,
    user_id: &UserId,
    nip: &Nip,
) -> Result<Option<NipAccount>, RepositoryError> {
    let row: Option<NipAccountRow> = sqlx::query_as(
        r"SELECT na.*
        FROM nip_accounts na
        INNER JOIN user_nip_access una ON una.nip_account_id = na.id
        WHERE una.user_id = $1 AND na.nip = $2",
    )
    .bind(user_id.as_uuid())
    .bind(nip.as_str())
    .fetch_optional(exec)
    .await?;

    row.map(NipAccountRow::into_domain).transpose()
}
