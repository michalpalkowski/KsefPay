use async_trait::async_trait;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::auth::TokenPair;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::nip::Nip;
use crate::domain::session::SessionReference;
use crate::error::RepositoryError;

/// Persistent record of auth tokens — defined here (not in domain)
/// because it's a storage concern, not a business concept.
#[derive(Debug, Clone)]
pub struct StoredTokenPair {
    pub id: Uuid,
    pub nip: Nip,
    pub environment: KSeFEnvironment,
    pub token_pair: TokenPair,
    pub created_at: DateTime<Utc>,
}

/// Persistent record of a `KSeF` session — defined here (not in domain)
/// because it's a storage concern, not a business concept.
#[derive(Debug, Clone)]
pub struct StoredSession {
    pub id: Uuid,
    pub session_reference: SessionReference,
    pub nip: Nip,
    pub environment: KSeFEnvironment,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub terminated_at: Option<DateTime<Utc>>,
}

/// Port: `KSeF` auth token and session persistence.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn save_token_pair(&self, token: &StoredTokenPair) -> Result<(), RepositoryError>;

    async fn find_active_token(
        &self,
        nip: &Nip,
        environment: KSeFEnvironment,
    ) -> Result<Option<StoredTokenPair>, RepositoryError>;

    async fn save_session(&self, session: &StoredSession) -> Result<(), RepositoryError>;

    async fn find_active_session(
        &self,
        nip: &Nip,
        environment: KSeFEnvironment,
    ) -> Result<Option<StoredSession>, RepositoryError>;

    async fn terminate_session(&self, session_id: Uuid) -> Result<(), RepositoryError>;
}
