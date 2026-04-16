use async_trait::async_trait;

use crate::domain::audit::{AuditLogEntry, NewAuditLogEntry};
use crate::error::RepositoryError;

/// Port: append-only audit logging.
#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    /// Persist a single audit entry.
    async fn log(&self, entry: &NewAuditLogEntry) -> Result<(), RepositoryError>;

    /// Read newest entries first (for diagnostics/admin tooling).
    async fn list_recent(&self, limit: u32) -> Result<Vec<AuditLogEntry>, RepositoryError>;
}
