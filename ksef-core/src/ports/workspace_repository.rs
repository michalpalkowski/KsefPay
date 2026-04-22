use async_trait::async_trait;

use crate::domain::account_scope::AccountScope;
use crate::domain::nip::Nip;
use crate::domain::nip_account::{NipAccount, NipAccountId};
use crate::domain::user::UserId;
use crate::domain::workspace::{
    Workspace, WorkspaceId, WorkspaceInvite, WorkspaceInviteId, WorkspaceMembership,
    WorkspaceNipOwnership, WorkspaceRole, WorkspaceSummary,
};
use crate::error::RepositoryError;

#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    async fn create_workspace(
        &self,
        workspace: &Workspace,
        owner_id: &UserId,
    ) -> Result<WorkspaceId, RepositoryError>;

    async fn ensure_default_workspace(
        &self,
        user_id: &UserId,
        user_email: &str,
    ) -> Result<WorkspaceSummary, RepositoryError>;

    async fn find_by_id(&self, workspace_id: &WorkspaceId) -> Result<Workspace, RepositoryError>;

    async fn list_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<Vec<WorkspaceSummary>, RepositoryError>;

    async fn find_membership(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
    ) -> Result<Option<WorkspaceMembership>, RepositoryError>;

    async fn add_member(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
        role: WorkspaceRole,
    ) -> Result<(), RepositoryError>;

    async fn attach_nip(
        &self,
        workspace_id: &WorkspaceId,
        account_id: &NipAccountId,
        ownership: WorkspaceNipOwnership,
        attached_by: &UserId,
    ) -> Result<(), RepositoryError>;

    async fn list_nip_accounts_for_user(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
    ) -> Result<Vec<NipAccount>, RepositoryError>;

    async fn find_user_account_in_workspace(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
        nip: &Nip,
    ) -> Result<Option<(NipAccount, AccountScope, WorkspaceMembership)>, RepositoryError>;

    async fn create_invite(
        &self,
        invite: &WorkspaceInvite,
    ) -> Result<WorkspaceInviteId, RepositoryError>;

    async fn list_pending_invites(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceInvite>, RepositoryError>;

    async fn find_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<WorkspaceInvite>, RepositoryError>;

    async fn activate_invite_membership(
        &self,
        invite: &WorkspaceInvite,
        user_id: &UserId,
    ) -> Result<(), RepositoryError>;

    async fn accept_invite(&self, invite_id: &WorkspaceInviteId) -> Result<(), RepositoryError>;

    async fn revoke_invite(&self, invite_id: &WorkspaceInviteId) -> Result<(), RepositoryError>;
}
