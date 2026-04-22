use chrono::Utc;
use thiserror::Error;

use ksef_core::domain::workspace::WorkspaceInvite;
use ksef_core::error::RepositoryError;

use crate::invite_tokens::hash_invite_token;
use crate::state::AppState;

#[derive(Debug, Error)]
pub enum InviteResolutionError {
    #[error("invite not found")]
    NotFound,
    #[error("invite already accepted")]
    AlreadyAccepted,
    #[error("invite revoked")]
    Revoked,
    #[error("invite expired")]
    Expired,
    #[error("invite email mismatch: expected {expected}, got {actual}")]
    EmailMismatch { expected: String, actual: String },
    #[error("workspace repository error: {0}")]
    Repository(#[from] RepositoryError),
}

pub async fn resolve_pending_invite(
    state: &AppState,
    raw_token: &str,
) -> Result<WorkspaceInvite, InviteResolutionError> {
    let token_hash = hash_invite_token(raw_token);
    let invite = state
        .workspace_repo
        .find_invite_by_token_hash(&token_hash)
        .await?
        .ok_or(InviteResolutionError::NotFound)?;

    if invite.accepted_at.is_some() {
        return Err(InviteResolutionError::AlreadyAccepted);
    }
    if invite.revoked_at.is_some() {
        return Err(InviteResolutionError::Revoked);
    }
    if invite.expires_at < Utc::now() {
        return Err(InviteResolutionError::Expired);
    }

    Ok(invite)
}

pub fn require_invite_email(
    invite: &WorkspaceInvite,
    actual_email: &str,
) -> Result<(), InviteResolutionError> {
    let normalized = actual_email.trim().to_lowercase();
    let expected = invite.email.trim().to_lowercase();

    if normalized == expected {
        return Ok(());
    }

    Err(InviteResolutionError::EmailMismatch {
        expected: invite.email.clone(),
        actual: actual_email.to_string(),
    })
}
