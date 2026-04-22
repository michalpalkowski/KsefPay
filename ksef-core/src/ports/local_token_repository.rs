use async_trait::async_trait;

use crate::domain::account_scope::AccountScope;
use crate::domain::token_mgmt::LocalToken;
use crate::domain::user::UserId;
use crate::error::RepositoryError;

/// Port: local token registry, scoped to NIP account and user.
#[async_trait]
pub trait LocalTokenRepository: Send + Sync {
    /// Persist a newly generated token entry.
    async fn save(&self, token: &LocalToken) -> Result<(), RepositoryError>;

    /// List all token entries for a given NIP account, newest first.
    async fn list_by_account(
        &self,
        scope: &AccountScope,
    ) -> Result<Vec<LocalToken>, RepositoryError>;

    /// List token entries for a specific `(NIP account, user)` pair, newest first.
    async fn list_by_account_for_user(
        &self,
        scope: &AccountScope,
        user_id: &UserId,
    ) -> Result<Vec<LocalToken>, RepositoryError>;

    /// Mark a token as revoked by its KSeF token ID, scoped to the given account.
    async fn mark_revoked(
        &self,
        ksef_token_id: &str,
        scope: &AccountScope,
    ) -> Result<(), RepositoryError>;
}
