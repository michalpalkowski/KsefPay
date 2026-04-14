use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::environment::KSeFEnvironment;
use crate::domain::nip::Nip;
use crate::error::RepositoryError;
use crate::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};

/// In-memory mock of `SessionRepository` for unit tests.
pub struct MockSessionRepo {
    tokens: Mutex<Vec<StoredTokenPair>>,
    sessions: Mutex<Vec<StoredSession>>,
}

impl MockSessionRepo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
        }
    }
}

impl Default for MockSessionRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SessionRepository for MockSessionRepo {
    async fn save_token_pair(&self, token: &StoredTokenPair) -> Result<(), RepositoryError> {
        let mut store = self.tokens.lock().unwrap();
        store.push(token.clone());
        Ok(())
    }

    async fn find_active_token(
        &self,
        nip: &Nip,
        environment: KSeFEnvironment,
    ) -> Result<Option<StoredTokenPair>, RepositoryError> {
        let store = self.tokens.lock().unwrap();
        let result = store
            .iter()
            .rev()
            .find(|t| t.nip.as_str() == nip.as_str() && t.environment == environment)
            .filter(|t| !t.token_pair.is_refresh_expired())
            .cloned();
        Ok(result)
    }

    async fn save_session(&self, session: &StoredSession) -> Result<(), RepositoryError> {
        let mut store = self.sessions.lock().unwrap();
        store.push(session.clone());
        Ok(())
    }

    async fn find_active_session(
        &self,
        nip: &Nip,
        environment: KSeFEnvironment,
    ) -> Result<Option<StoredSession>, RepositoryError> {
        let store = self.sessions.lock().unwrap();
        let result = store
            .iter()
            .rev()
            .find(|s| {
                s.nip.as_str() == nip.as_str()
                    && s.environment == environment
                    && s.terminated_at.is_none()
            })
            .cloned();
        Ok(result)
    }

    async fn terminate_session(&self, session_id: uuid::Uuid) -> Result<(), RepositoryError> {
        let mut store = self.sessions.lock().unwrap();
        let session = store
            .iter_mut()
            .find(|s| s.id == session_id)
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Session",
                id: session_id.to_string(),
            })?;
        session.terminated_at = Some(chrono::Utc::now());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::domain::auth::{AccessToken, RefreshToken, TokenPair};
    use crate::domain::session::SessionReference;

    fn test_nip() -> Nip {
        Nip::parse("5260250274").unwrap()
    }

    fn make_token_pair(access_mins: i64, refresh_days: i64) -> TokenPair {
        TokenPair {
            access_token: AccessToken::new("access".to_string()),
            refresh_token: RefreshToken::new("refresh".to_string()),
            access_token_expires_at: Utc::now() + chrono::Duration::minutes(access_mins),
            refresh_token_expires_at: Utc::now() + chrono::Duration::days(refresh_days),
        }
    }

    fn make_stored_token(env: KSeFEnvironment) -> StoredTokenPair {
        StoredTokenPair {
            id: Uuid::new_v4(),
            nip: test_nip(),
            environment: env,
            token_pair: make_token_pair(15, 7),
            created_at: Utc::now(),
        }
    }

    fn make_stored_session(env: KSeFEnvironment) -> StoredSession {
        StoredSession {
            id: Uuid::new_v4(),
            session_reference: SessionReference::new("session-ref-123".to_string()),
            nip: test_nip(),
            environment: env,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(12),
            terminated_at: None,
        }
    }

    /// Contract test: save and find active token.
    #[tokio::test]
    async fn save_and_find_active_token() {
        let repo = MockSessionRepo::new();
        let token = make_stored_token(KSeFEnvironment::Test);
        repo.save_token_pair(&token).await.unwrap();

        let found = repo
            .find_active_token(&test_nip(), KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(found.is_some());
    }

    /// Contract test: find_active_token returns None for wrong environment.
    #[tokio::test]
    async fn find_active_token_wrong_env_returns_none() {
        let repo = MockSessionRepo::new();
        let token = make_stored_token(KSeFEnvironment::Test);
        repo.save_token_pair(&token).await.unwrap();

        let found = repo
            .find_active_token(&test_nip(), KSeFEnvironment::Production)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    /// Contract test: expired refresh token is not returned.
    #[tokio::test]
    async fn expired_refresh_token_not_returned() {
        let repo = MockSessionRepo::new();
        let mut token = make_stored_token(KSeFEnvironment::Test);
        token.token_pair = TokenPair {
            access_token: AccessToken::new("a".to_string()),
            refresh_token: RefreshToken::new("r".to_string()),
            access_token_expires_at: Utc::now() - chrono::Duration::hours(1),
            refresh_token_expires_at: Utc::now() - chrono::Duration::days(1),
        };
        repo.save_token_pair(&token).await.unwrap();

        let found = repo
            .find_active_token(&test_nip(), KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    /// Contract test: save and find active session.
    #[tokio::test]
    async fn save_and_find_active_session() {
        let repo = MockSessionRepo::new();
        let session = make_stored_session(KSeFEnvironment::Test);
        repo.save_session(&session).await.unwrap();

        let found = repo
            .find_active_session(&test_nip(), KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().session_reference.as_str(), "session-ref-123");
    }

    /// Contract test: terminated session is not returned as active.
    #[tokio::test]
    async fn terminated_session_not_active() {
        let repo = MockSessionRepo::new();
        let session = make_stored_session(KSeFEnvironment::Test);
        let session_id = session.id;
        repo.save_session(&session).await.unwrap();

        repo.terminate_session(session_id).await.unwrap();

        let found = repo
            .find_active_session(&test_nip(), KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    /// Contract test: terminate missing session returns NotFound.
    #[tokio::test]
    async fn terminate_missing_session_returns_error() {
        let repo = MockSessionRepo::new();
        let err = repo.terminate_session(Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }
}
