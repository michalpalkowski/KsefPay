use sqlx::PgExecutor;

use crate::domain::company::{CompanyInfo, VatStatus};
use crate::domain::nip::Nip;
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct CompanyCacheRow {
    nip: String,
    name: String,
    address: String,
    bank_accounts: String,
    vat_status: String,
    fetched_at: chrono::DateTime<chrono::Utc>,
}

impl CompanyCacheRow {
    fn into_domain(self) -> Result<CompanyInfo, RepositoryError> {
        let nip = Nip::parse(&self.nip).map_err(|e| RepositoryError::Database(sqlx::Error::Decode(
            format!("invalid NIP in company_cache: {e}").into(),
        )))?;
        let bank_accounts: Vec<String> = serde_json::from_str(&self.bank_accounts)
            .unwrap_or_default();
        Ok(CompanyInfo {
            nip,
            name: self.name,
            address: self.address,
            bank_accounts,
            vat_status: VatStatus::from_whitelist(&self.vat_status),
            fetched_at: self.fetched_at,
        })
    }
}

pub async fn get<'e>(
    exec: impl PgExecutor<'e>,
    nip: &Nip,
) -> Result<Option<CompanyInfo>, RepositoryError> {
    let row: Option<CompanyCacheRow> = sqlx::query_as(
        "SELECT nip, name, address, bank_accounts, vat_status, fetched_at FROM company_cache WHERE nip = $1",
    )
    .bind(nip.as_str())
    .fetch_optional(exec)
    .await?;
    row.map(CompanyCacheRow::into_domain).transpose()
}

pub async fn set<'e>(
    exec: impl PgExecutor<'e>,
    info: &CompanyInfo,
) -> Result<(), RepositoryError> {
    let bank_accounts_json = serde_json::to_string(&info.bank_accounts)
        .unwrap_or_else(|_| "[]".to_string());
    sqlx::query(
        r"INSERT INTO company_cache (nip, name, address, bank_accounts, vat_status, fetched_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (nip) DO UPDATE SET
            name = EXCLUDED.name,
            address = EXCLUDED.address,
            bank_accounts = EXCLUDED.bank_accounts,
            vat_status = EXCLUDED.vat_status,
            fetched_at = EXCLUDED.fetched_at",
    )
    .bind(info.nip.as_str())
    .bind(&info.name)
    .bind(&info.address)
    .bind(&bank_accounts_json)
    .bind(info.vat_status.to_string())
    .bind(info.fetched_at)
    .execute(exec)
    .await?;
    Ok(())
}
