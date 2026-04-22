use async_trait::async_trait;

use crate::domain::application_access::{ApplicationAccessInvite, ApplicationAccessInviteId};
use crate::error::RepositoryError;

#[async_trait]
pub trait ApplicationAccessRepository: Send + Sync {
    async fn create_invite(
        &self,
        invite: &ApplicationAccessInvite,
    ) -> Result<ApplicationAccessInviteId, RepositoryError>;

    async fn list_pending_invites(&self) -> Result<Vec<ApplicationAccessInvite>, RepositoryError>;

    async fn find_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApplicationAccessInvite>, RepositoryError>;

    async fn accept_invite(
        &self,
        invite_id: &ApplicationAccessInviteId,
    ) -> Result<(), RepositoryError>;

    async fn revoke_invite(
        &self,
        invite_id: &ApplicationAccessInviteId,
    ) -> Result<(), RepositoryError>;
}
