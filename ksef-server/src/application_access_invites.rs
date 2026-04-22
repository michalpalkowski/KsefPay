use chrono::Utc;
use thiserror::Error;

use ksef_core::domain::application_access::ApplicationAccessInvite;
use ksef_core::error::RepositoryError;

use crate::invite_tokens::hash_invite_token;
use crate::state::AppState;

#[derive(Debug, Error)]
pub enum ApplicationAccessInviteResolutionError {
    #[error("application access invite not found")]
    NotFound,
    #[error("application access invite already accepted")]
    AlreadyAccepted,
    #[error("application access invite revoked")]
    Revoked,
    #[error("application access invite expired")]
    Expired,
    #[error("application access invite email mismatch: expected {expected}, got {actual}")]
    EmailMismatch { expected: String, actual: String },
    #[error("application access repository error: {0}")]
    Repository(#[from] RepositoryError),
}

pub async fn resolve_pending_application_access_invite(
    state: &AppState,
    raw_token: &str,
) -> Result<ApplicationAccessInvite, ApplicationAccessInviteResolutionError> {
    let token_hash = hash_invite_token(raw_token);
    let invite = state
        .application_access_repo
        .find_invite_by_token_hash(&token_hash)
        .await?
        .ok_or(ApplicationAccessInviteResolutionError::NotFound)?;

    if invite.accepted_at.is_some() {
        return Err(ApplicationAccessInviteResolutionError::AlreadyAccepted);
    }
    if invite.revoked_at.is_some() {
        return Err(ApplicationAccessInviteResolutionError::Revoked);
    }
    if invite.expires_at < Utc::now() {
        return Err(ApplicationAccessInviteResolutionError::Expired);
    }

    Ok(invite)
}

pub fn require_application_access_invite_email(
    invite: &ApplicationAccessInvite,
    actual_email: &str,
) -> Result<(), ApplicationAccessInviteResolutionError> {
    let normalized = actual_email.trim().to_lowercase();
    let expected = invite.email.trim().to_lowercase();

    if normalized == expected {
        return Ok(());
    }

    Err(ApplicationAccessInviteResolutionError::EmailMismatch {
        expected: invite.email.clone(),
        actual: actual_email.to_string(),
    })
}
