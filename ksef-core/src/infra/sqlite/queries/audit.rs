use sqlx::SqliteExecutor;

use super::datetime::parse_sqlite_datetime;
use crate::domain::audit::{AuditAction, AuditLogEntry, NewAuditLogEntry};
use crate::domain::nip::Nip;
use crate::domain::user::UserId;
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: String,
    timestamp: String,
    user_id: String,
    user_email: String,
    nip: Option<String>,
    action: String,
    details: Option<String>,
    ip_address: Option<String>,
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

impl AuditRow {
    fn into_domain(self) -> Result<AuditLogEntry, RepositoryError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| decode_err(format!("invalid audit id '{}': {e}", self.id)))?;

        let user_uuid = uuid::Uuid::parse_str(&self.user_id)
            .map_err(|e| decode_err(format!("invalid user_id '{}': {e}", self.user_id)))?;

        let nip = self
            .nip
            .map(|value| {
                Nip::parse(&value)
                    .map_err(|e| decode_err(format!("invalid audit NIP '{value}': {e}")))
            })
            .transpose()?;

        let action = self
            .action
            .parse::<AuditAction>()
            .map_err(|e| decode_err(format!("invalid audit action '{}': {e}", self.action)))?;

        Ok(AuditLogEntry {
            id,
            timestamp: parse_datetime(&self.timestamp, "timestamp")?,
            user_id: UserId::from_uuid(user_uuid),
            user_email: self.user_email,
            nip,
            action,
            details: self.details,
            ip_address: self.ip_address,
        })
    }
}

pub async fn log<'e>(
    exec: impl SqliteExecutor<'e>,
    entry: &NewAuditLogEntry,
) -> Result<(), RepositoryError> {
    sqlx::query(
        r"INSERT INTO audit_log (
            id, timestamp, user_id, user_email, nip, action, details, ip_address
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(entry.user_id.to_string())
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
    exec: impl SqliteExecutor<'e>,
    limit: u32,
) -> Result<Vec<AuditLogEntry>, RepositoryError> {
    let rows: Vec<AuditRow> = sqlx::query_as(
        r"SELECT id, timestamp, user_id, user_email, nip, action, details, ip_address
           FROM audit_log
           ORDER BY timestamp DESC
           LIMIT ?1",
    )
    .bind(i64::from(limit))
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(AuditRow::into_domain).collect()
}
