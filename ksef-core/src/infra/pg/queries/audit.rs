use sqlx::PgExecutor;

use crate::domain::audit::{AuditAction, AuditLogEntry, NewAuditLogEntry};
use crate::domain::nip::Nip;
use crate::domain::user::UserId;
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: uuid::Uuid,
    timestamp: chrono::DateTime<chrono::Utc>,
    user_id: uuid::Uuid,
    user_email: String,
    nip: Option<String>,
    action: String,
    details: Option<String>,
    ip_address: Option<String>,
}

impl AuditRow {
    fn into_domain(self) -> Result<AuditLogEntry, RepositoryError> {
        let nip = self
            .nip
            .map(|value| {
                Nip::parse(&value).map_err(|e| {
                    RepositoryError::Database(sqlx::Error::Decode(
                        format!("invalid audit NIP '{value}': {e}").into(),
                    ))
                })
            })
            .transpose()?;

        let action = self.action.parse::<AuditAction>().map_err(|e| {
            RepositoryError::Database(sqlx::Error::Decode(
                format!("invalid audit action '{}': {e}", self.action).into(),
            ))
        })?;

        Ok(AuditLogEntry {
            id: self.id,
            timestamp: self.timestamp,
            user_id: UserId::from_uuid(self.user_id),
            user_email: self.user_email,
            nip,
            action,
            details: self.details,
            ip_address: self.ip_address,
        })
    }
}

pub async fn log<'e>(
    exec: impl PgExecutor<'e>,
    entry: &NewAuditLogEntry,
) -> Result<(), RepositoryError> {
    sqlx::query(
        r"INSERT INTO audit_log (
            id, timestamp, user_id, user_email, nip, action, details, ip_address
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(chrono::Utc::now())
    .bind(entry.user_id.as_uuid())
    .bind(&entry.user_email)
    .bind(entry.nip.as_ref().map(ToString::to_string))
    .bind(entry.action.to_string())
    .bind(entry.details.clone())
    .bind(entry.ip_address.clone())
    .execute(exec)
    .await?;

    Ok(())
}

pub async fn list_recent<'e>(
    exec: impl PgExecutor<'e>,
    limit: u32,
) -> Result<Vec<AuditLogEntry>, RepositoryError> {
    let rows: Vec<AuditRow> = sqlx::query_as(
        r"SELECT id, timestamp, user_id, user_email, nip, action, details, ip_address
           FROM audit_log
           ORDER BY timestamp DESC
           LIMIT $1",
    )
    .bind(i64::from(limit))
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(AuditRow::into_domain).collect()
}
