use async_trait::async_trait;

use crate::domain::auth::{AccessToken, AuthSessionInfo};
use crate::error::KSeFError;

/// Port: currently authenticated `KSeF` auth sessions management.
#[async_trait]
pub trait KSeFAuthSessions: Send + Sync {
    async fn list_sessions(
        &self,
        access_token: &AccessToken,
    ) -> Result<Vec<AuthSessionInfo>, KSeFError>;

    async fn revoke_session(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<(), KSeFError>;

    async fn revoke_current_session(&self, access_token: &AccessToken) -> Result<(), KSeFError>;
}
