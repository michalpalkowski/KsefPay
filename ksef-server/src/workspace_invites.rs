use chrono::{Duration, Utc};
use openssl::base64::encode_block;
use openssl::sha::sha256;
use thiserror::Error;
use uuid::Uuid;

use ksef_core::domain::workspace::WorkspaceInvite;
use ksef_core::error::RepositoryError;

use crate::state::AppState;

const INVITE_TTL_DAYS: i64 = 7;

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

pub fn generate_invite_token() -> String {
    format!("{}.{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn hash_invite_token(raw_token: &str) -> String {
    encode_block(&sha256(raw_token.as_bytes()))
}

pub fn invite_expiration() -> chrono::DateTime<Utc> {
    Utc::now() + Duration::days(INVITE_TTL_DAYS)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_url_safe() {
        let token = generate_invite_token();
        assert!(token.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '.'));
    }

    #[test]
    fn hash_is_deterministic() {
        let a = hash_invite_token("abc");
        let b = hash_invite_token("abc");
        assert_eq!(a, b);
    }
}
