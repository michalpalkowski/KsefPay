use async_trait::async_trait;

use crate::domain::nip::Nip;
use crate::domain::nip_account::{NipAccount, NipAccountId};
use crate::error::RepositoryError;

/// Port: NIP account persistence.
#[async_trait]
pub trait NipAccountRepository: Send + Sync {
    /// Create a new NIP account. Returns `Duplicate` if NIP already registered.
    async fn create(&self, account: &NipAccount) -> Result<NipAccountId, RepositoryError>;

    /// Find NIP account by ID. Returns `NotFound` if missing.
    async fn find_by_id(&self, id: &NipAccountId) -> Result<NipAccount, RepositoryError>;

    /// Find NIP account by NIP. Returns `None` if not registered.
    async fn find_by_nip(&self, nip: &Nip) -> Result<Option<NipAccount>, RepositoryError>;

    /// Update KSeF credentials (cert, key, auth method, auth token) for an account.
    async fn update_credentials(&self, account: &NipAccount) -> Result<(), RepositoryError>;
}
