use sqlx::PgExecutor;

use crate::domain::nip::Nip;
use crate::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
use crate::error::RepositoryError;
use crate::infra::crypto::CertificateSecretBox;

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

#[derive(sqlx::FromRow)]
pub(crate) struct NipAccountRow {
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
    pub(crate) fn into_domain(
        self,
        certificate_secret_box: &CertificateSecretBox,
    ) -> Result<NipAccount, RepositoryError> {
        let nip = Nip::parse(&self.nip)
            .map_err(|e| decode_err(format!("invalid NIP in nip_accounts: {e}")))?;
        let ksef_auth_method: KSeFAuthMethod = self
            .ksef_auth_method
            .parse()
            .map_err(|e: String| decode_err(e))?;
        let cert_pem = decode_secret(self.cert_pem, certificate_secret_box, "cert_pem")?;
        let key_pem = decode_secret(self.key_pem, certificate_secret_box, "key_pem")?;

        Ok(NipAccount {
            id: NipAccountId::from_uuid(self.id),
            nip,
            display_name: self.display_name,
            ksef_auth_method,
            ksef_auth_token: self.ksef_auth_token,
            cert_pem,
            key_pem,
            cert_auto_generated: self.cert_auto_generated,
            created_at: self.created_at,
            updated_at: self.updated_at,
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
    exec: impl PgExecutor<'e>,
    account: &NipAccount,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<NipAccountId, RepositoryError> {
    let cert_pem_str = encode_secret(&account.cert_pem, certificate_secret_box, "cert_pem")?;
    let key_pem_str = encode_secret(&account.key_pem, certificate_secret_box, "key_pem")?;

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
    certificate_secret_box: &CertificateSecretBox,
) -> Result<NipAccount, RepositoryError> {
    let row: NipAccountRow = sqlx::query_as("SELECT * FROM nip_accounts WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_optional(exec)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "NipAccount",
            id: id.to_string(),
        })?;
    row.into_domain(certificate_secret_box)
}

pub async fn find_by_nip<'e>(
    exec: impl PgExecutor<'e>,
    nip: &Nip,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<Option<NipAccount>, RepositoryError> {
    let row: Option<NipAccountRow> = sqlx::query_as("SELECT * FROM nip_accounts WHERE nip = $1")
        .bind(nip.as_str())
        .fetch_optional(exec)
        .await?;
    row.map(|row| row.into_domain(certificate_secret_box))
        .transpose()
}

pub async fn update_credentials<'e>(
    exec: impl PgExecutor<'e>,
    account: &NipAccount,
    certificate_secret_box: &CertificateSecretBox,
) -> Result<(), RepositoryError> {
    let cert_pem_str = encode_secret(&account.cert_pem, certificate_secret_box, "cert_pem")?;
    let key_pem_str = encode_secret(&account.key_pem, certificate_secret_box, "key_pem")?;

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
