use async_trait::async_trait;

use crate::domain::auth::{
    AuthChallenge, AuthReference, AuthStatus, ContextIdentifier, RefreshToken, TokenPair,
};
use crate::domain::crypto::SignedAuthRequest;
use crate::domain::nip::Nip;
use crate::error::KSeFError;

/// Port: `KSeF` authentication (challenge -> sign -> redeem JWT).
#[async_trait]
pub trait KSeFAuth: Send + Sync {
    async fn request_challenge(&self, nip: &Nip) -> Result<AuthChallenge, KSeFError>;

    async fn authenticate_xades(
        &self,
        signed_request: &SignedAuthRequest,
    ) -> Result<AuthReference, KSeFError>;

    async fn authenticate_token(
        &self,
        context: &ContextIdentifier,
        token: &str,
    ) -> Result<AuthReference, KSeFError>;

    async fn poll_auth_status(&self, reference: &AuthReference) -> Result<AuthStatus, KSeFError>;

    async fn redeem_token(&self, reference: &AuthReference) -> Result<TokenPair, KSeFError>;

    async fn refresh_token(&self, refresh_token: &RefreshToken) -> Result<TokenPair, KSeFError>;
}
