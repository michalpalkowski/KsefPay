use std::sync::Arc;

use crate::domain::audit::{AuditAction, AuditLogEntry, NewAuditLogEntry};
use crate::domain::nip::Nip;
use crate::domain::user::UserId;
use crate::error::RepositoryError;
use crate::ports::audit_log::AuditLogRepository;

/// Application service for writing structured audit events.
pub struct AuditService {
    repo: Arc<dyn AuditLogRepository>,
}

impl AuditService {
    #[must_use]
    pub fn new(repo: Arc<dyn AuditLogRepository>) -> Self {
        Self { repo }
    }

    /// Log one sensitive action.
    pub async fn log_action(
        &self,
        user_id: &UserId,
        user_email: &str,
        nip: Option<&Nip>,
        action: AuditAction,
        details: Option<String>,
        ip_address: Option<String>,
    ) -> Result<(), RepositoryError> {
        let entry = NewAuditLogEntry {
            user_id: user_id.clone(),
            user_email: user_email.to_string(),
            nip: nip.cloned(),
            action,
            details,
            ip_address,
        };

        self.repo.log(&entry).await
    }

    pub async fn list_recent(&self, limit: u32) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        self.repo.list_recent(limit).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use super::*;
    use crate::domain::audit::{AuditAction, AuditLogEntry, NewAuditLogEntry};
    use crate::domain::nip::Nip;
    use crate::domain::user::UserId;

    #[derive(Default)]
    struct MockAuditRepo {
        entries: Mutex<Vec<NewAuditLogEntry>>,
    }

    #[async_trait]
    impl AuditLogRepository for MockAuditRepo {
        async fn log(&self, entry: &NewAuditLogEntry) -> Result<(), RepositoryError> {
            self.entries.lock().await.push(entry.clone());
            Ok(())
        }

        async fn list_recent(&self, _limit: u32) -> Result<Vec<AuditLogEntry>, RepositoryError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn log_action_writes_expected_entry() {
        let repo = Arc::new(MockAuditRepo::default());
        let service = AuditService::new(repo.clone());

        let user_id = UserId::new();
        let nip = Nip::parse("5260250274").unwrap();

        service
            .log_action(
                &user_id,
                "user@example.com",
                Some(&nip),
                AuditAction::Login,
                Some("ok".to_string()),
                Some("203.0.113.1".to_string()),
            )
            .await
            .unwrap();

        let entries = repo.entries.lock().await;
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.user_id, user_id);
        assert_eq!(entry.user_email, "user@example.com");
        assert_eq!(entry.nip.as_ref(), Some(&nip));
        assert_eq!(entry.action, AuditAction::Login);
        assert_eq!(entry.details.as_deref(), Some("ok"));
        assert_eq!(entry.ip_address.as_deref(), Some("203.0.113.1"));
    }
}
