use sqlx::SqliteExecutor;

use super::datetime::parse_sqlite_datetime;
use crate::domain::account_scope::AccountScope;
use crate::domain::nip::Nip;
use crate::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
use crate::domain::user::UserId;
use crate::error::RepositoryError;
use crate::infra::crypto::CertificateSecretBox;

#[derive(sqlx::FromRow)]
pub(crate) struct NipAccountRow {
    pub id: String,
    pub nip: String,
    pub display_name: String,
    pub ksef_auth_method: String,
    pub ksef_auth_token: Option<String>,
    pub cert_pem: Option<String>,
    pub key_pem: Option<String>,
    pub cert_auto_generated: i32,
    pub created_at: String,
    pub updated_at: String,
}

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

fn parse_datetime(
    value: &str,
    field: &'static str,
) -> Result<chrono::DateTime<chrono::Utc>, RepositoryError> {
    parse_sqlite_datetime(value, field).map_err(decode_err)
}

fn decode_nip(s: &str) -> Result<Nip, RepositoryError> {
    Nip::parse(s).map_err(|e| decode_err(format!("invalid NIP in nip_account row: {e}")))
}

impl NipAccountRow {
    pub(crate) fn into_domain(
        self,
        certificate_secret_box: &CertificateSecretBox,
    ) -> Result<NipAccount, RepositoryError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| decode_err(format!("invalid nip_account id '{}': {e}", self.id)))?;

        let ksef_auth_method: KSeFAuthMethod = self
            .ksef_auth_method
            .parse()
            .map_err(|e: String| decode_err(e))?;

        let cert_pem = decode_secret(self.cert_pem, certificate_secret_box, "cert_pem")?;
        let key_pem = decode_secret(self.key_pem, certificate_secret_box, "key_pem")?;

        Ok(NipAccount {
            id: NipAccountId::from_uuid(id),
            nip: decode_nip(&self.nip)?,
            display_name: self.display_name,
            ksef_auth_method,
            ksef_auth_token: self.ksef_auth_token,
            cert_pem,
            key_pem,
            cert_auto_generated: self.cert_auto_generated != 0,
            created_at: parse_datetime(&self.created_at, "created_at")?,
            updated_at: parse_datetime(&self.updated_at, "updated_at")?,
        })
    }
}

fn encode_secret(
    secret: &Option<Vec<u8>>,
    certificate_secret_box: &CertificateSecretBox,
    field: &'static str,
) -> Result<Option<String>, RepositoryError> {
    secret
        .as_ref()
        .map(|bytes| {
            certificate_secret_box
                .encrypt(bytes)
                .map_err(|e| RepositoryError::Storage(format!("{field}: {e}")))
        })
        .transpose()
}

fn decode_secret(
    secret: Option<String>,
    certificate_secret_box: &CertificateSecretBox,
    field: &'static str,
) -> Result<Option<Vec<u8>>, RepositoryError> {
    secret
        .map(|value| {
            certificate_secret_box
                .decrypt_or_plaintext(&value)
                .map_err(|e| RepositoryError::Storage(format!("{field}: {e}")))
        })
        .transpose()
}

pub async fn create<'e>(
    exec: impl SqliteExecutor<'e>,
    account: &NipAccount,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<NipAccountId, RepositoryError> {
    let cert_pem = encode_secret(&account.cert_pem, certificate_secret_box, "cert_pem")?;
    let key_pem = encode_secret(&account.key_pem, certificate_secret_box, "key_pem")?;

    sqlx::query(
        r"INSERT INTO nip_accounts (
            id, nip, display_name, ksef_auth_method, ksef_auth_token,
            cert_pem, key_pem, cert_auto_generated, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(account.id.to_string())
    .bind(account.nip.as_str())
    .bind(&account.display_name)
    .bind(account.ksef_auth_method.to_string())
    .bind(&account.ksef_auth_token)
    .bind(&cert_pem)
    .bind(&key_pem)
    .bind(i32::from(account.cert_auto_generated))
    .bind(account.created_at.to_rfc3339())
    .bind(account.updated_at.to_rfc3339())
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
    exec: impl SqliteExecutor<'e>,
    id: &NipAccountId,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<NipAccount, RepositoryError> {
    let row: NipAccountRow = sqlx::query_as("SELECT * FROM nip_accounts WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(exec)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "NipAccount",
            id: id.to_string(),
        })?;
    row.into_domain(certificate_secret_box)
}

pub async fn find_by_nip<'e>(
    exec: impl SqliteExecutor<'e>,
    nip: &Nip,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<Option<NipAccount>, RepositoryError> {
    let row: Option<NipAccountRow> = sqlx::query_as("SELECT * FROM nip_accounts WHERE nip = ?1")
        .bind(nip.as_str())
        .fetch_optional(exec)
        .await?;
    row.map(|row| row.into_domain(certificate_secret_box))
        .transpose()
}

pub async fn update_credentials<'e>(
    exec: impl SqliteExecutor<'e>,
    account: &NipAccount,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<(), RepositoryError> {
    let cert_pem = encode_secret(&account.cert_pem, certificate_secret_box, "cert_pem")?;
    let key_pem = encode_secret(&account.key_pem, certificate_secret_box, "key_pem")?;

    let result = sqlx::query(
        r"UPDATE nip_accounts SET
            ksef_auth_method = ?1,
            ksef_auth_token = ?2,
            cert_pem = ?3,
            key_pem = ?4,
            cert_auto_generated = ?5,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
        WHERE id = ?6",
    )
    .bind(account.ksef_auth_method.to_string())
    .bind(&account.ksef_auth_token)
    .bind(&cert_pem)
    .bind(&key_pem)
    .bind(i32::from(account.cert_auto_generated))
    .bind(account.id.to_string())
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
    exec: impl SqliteExecutor<'e>,
    user_id: &UserId,
    account_id: &NipAccountId,
    can_manage_credentials: bool,
) -> Result<(), RepositoryError> {
    sqlx::query(
        r"INSERT INTO user_nip_access (user_id, nip_account_id, can_manage_credentials)
          VALUES (?1, ?2, ?3)
          ON CONFLICT(user_id, nip_account_id) DO UPDATE SET
            can_manage_credentials = excluded.can_manage_credentials",
    )
    .bind(user_id.to_string())
    .bind(account_id.to_string())
    .bind(i32::from(can_manage_credentials))
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
    exec: impl SqliteExecutor<'e>,
    user_id: &UserId,
    account_id: &NipAccountId,
) -> Result<(), RepositoryError> {
    let result =
        sqlx::query(r"DELETE FROM user_nip_access WHERE user_id = ?1 AND nip_account_id = ?2")
            .bind(user_id.to_string())
            .bind(account_id.to_string())
            .execute(exec)
            .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "UserNipAccess",
            id: format!("{}:{}", user_id, account_id),
        });
    }
    Ok(())
}

pub async fn list_by_user<'e>(
    exec: impl SqliteExecutor<'e>,
    user_id: &UserId,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<Vec<NipAccount>, RepositoryError> {
    let rows: Vec<NipAccountRow> = sqlx::query_as(
        r"SELECT na.* FROM nip_accounts na
           INNER JOIN user_nip_access una ON una.nip_account_id = na.id
           WHERE una.user_id = ?1
           ORDER BY na.display_name",
    )
    .bind(user_id.to_string())
    .fetch_all(exec)
    .await?;

    rows.into_iter()
        .map(|row| row.into_domain(certificate_secret_box))
        .collect()
}

pub async fn verify_access<'e>(
    exec: impl SqliteExecutor<'e>,
    user_id: &UserId,
    nip: &Nip,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<Option<(NipAccount, AccountScope)>, RepositoryError> {
    let row: Option<NipAccountRow> = sqlx::query_as(
        r"SELECT na.* FROM nip_accounts na
           INNER JOIN user_nip_access una ON una.nip_account_id = na.id
           WHERE una.user_id = ?1 AND na.nip = ?2",
    )
    .bind(user_id.to_string())
    .bind(nip.as_str())
    .fetch_optional(exec)
    .await?;

    row.map(|r| {
        let account = r.into_domain(certificate_secret_box)?;
        let scope = AccountScope::new(account.id.clone(), account.nip.clone());
        Ok((account, scope))
    })
    .transpose()
}

pub async fn can_manage_credentials<'e>(
    exec: impl SqliteExecutor<'e>,
    user_id: &UserId,
    account_id: &NipAccountId,
) -> Result<bool, RepositoryError> {
    let allowed = sqlx::query_scalar::<_, Option<i32>>(
        "SELECT can_manage_credentials FROM user_nip_access WHERE user_id = ?1 AND nip_account_id = ?2",
    )
    .bind(user_id.to_string())
    .bind(account_id.to_string())
    .fetch_optional(exec)
    .await?
    .flatten()
    .unwrap_or(0);

    Ok(allowed != 0)
}
