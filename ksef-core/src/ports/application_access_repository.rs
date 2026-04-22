use async_trait::async_trait;

use crate::domain::application_access::{
    ApplicationAccessInvite, ApplicationAccessInviteId, TrustedApplicationEmailAccess,
    TrustedApplicationEmailAccessId,
};
use crate::domain::user::UserId;
use crate::domain::workspace::WorkspaceSummary;
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

    async fn activate_application_access(
        &self,
        invite_id: &ApplicationAccessInviteId,
        user_id: &UserId,
        user_email: &str,
    ) -> Result<WorkspaceSummary, RepositoryError>;

    async fn accept_invite(
        &self,
        invite_id: &ApplicationAccessInviteId,
    ) -> Result<(), RepositoryError>;

    async fn revoke_invite(
        &self,
        invite_id: &ApplicationAccessInviteId,
    ) -> Result<(), RepositoryError>;

    async fn create_trusted_email_access(
        &self,
        access: &TrustedApplicationEmailAccess,
    ) -> Result<TrustedApplicationEmailAccessId, RepositoryError>;

    async fn list_pending_trusted_email_access(
        &self,
    ) -> Result<Vec<TrustedApplicationEmailAccess>, RepositoryError>;

    async fn find_pending_trusted_email_access_by_email(
        &self,
        email: &str,
    ) -> Result<Option<TrustedApplicationEmailAccess>, RepositoryError>;

    async fn activate_trusted_email_access(
        &self,
        access_id: &TrustedApplicationEmailAccessId,
        user_id: &UserId,
        user_email: &str,
    ) -> Result<WorkspaceSummary, RepositoryError>;

    async fn revoke_trusted_email_access(
        &self,
        access_id: &TrustedApplicationEmailAccessId,
    ) -> Result<(), RepositoryError>;
}
