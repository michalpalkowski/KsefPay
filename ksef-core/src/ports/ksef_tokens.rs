use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::auth::AccessToken;
use crate::domain::permission::PermissionType;
use crate::domain::token_mgmt::{ManagedToken, TokenStatus};
use crate::error::KSeFError;

#[derive(Debug, Clone)]
pub struct TokenGenerateRequest {
    pub permissions: Vec<PermissionType>,
    pub description: Option<String>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TokenQueryRequest {
    pub status: Option<TokenStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TokenQueryResponse {
    pub items: Vec<ManagedToken>,
    pub total: u32,
}

/// Port: `KSeF` token management.
#[async_trait]
pub trait KSeFTokens: Send + Sync {
    async fn generate_token(
        &self,
        access_token: &AccessToken,
        request: &TokenGenerateRequest,
    ) -> Result<ManagedToken, KSeFError>;

    async fn query_tokens(
        &self,
        access_token: &AccessToken,
        request: &TokenQueryRequest,
    ) -> Result<TokenQueryResponse, KSeFError>;

    async fn get_token(
        &self,
        access_token: &AccessToken,
        token_id: &str,
    ) -> Result<ManagedToken, KSeFError>;

    async fn revoke_token(
        &self,
        access_token: &AccessToken,
        token_id: &str,
    ) -> Result<(), KSeFError>;
}
