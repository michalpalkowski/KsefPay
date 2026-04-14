use std::sync::Arc;

use crate::domain::auth::AccessToken;
use crate::domain::token_mgmt::ManagedToken;
use crate::error::{DomainError, KSeFError};
use crate::ports::ksef_tokens::{
    KSeFTokens, TokenGenerateRequest, TokenQueryRequest, TokenQueryResponse,
};

pub struct TokenMgmtService {
    port: Arc<dyn KSeFTokens>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenMgmtServiceError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error(transparent)]
    KSeF(#[from] KSeFError),
}

impl TokenMgmtService {
    #[must_use]
    pub fn new(port: Arc<dyn KSeFTokens>) -> Self {
        Self { port }
    }

    pub async fn generate(
        &self,
        access_token: &AccessToken,
        request: &TokenGenerateRequest,
    ) -> Result<ManagedToken, TokenMgmtServiceError> {
        if request.permissions.is_empty() {
            return Err(DomainError::InvalidParse {
                type_name: "TokenGenerateRequest.permissions",
                value: "empty".to_string(),
            }
            .into());
        }

        Ok(self.port.generate_token(access_token, request).await?)
    }

    pub async fn query(
        &self,
        access_token: &AccessToken,
        request: &TokenQueryRequest,
    ) -> Result<TokenQueryResponse, TokenMgmtServiceError> {
        Ok(self.port.query_tokens(access_token, request).await?)
    }

    pub async fn get(
        &self,
        access_token: &AccessToken,
        token_id: &str,
    ) -> Result<ManagedToken, TokenMgmtServiceError> {
        if token_id.trim().is_empty() {
            return Err(DomainError::InvalidParse {
                type_name: "TokenId",
                value: token_id.to_string(),
            }
            .into());
        }
        Ok(self.port.get_token(access_token, token_id).await?)
    }

    pub async fn revoke(
        &self,
        access_token: &AccessToken,
        token_id: &str,
    ) -> Result<(), TokenMgmtServiceError> {
        if token_id.trim().is_empty() {
            return Err(DomainError::InvalidParse {
                type_name: "TokenId",
                value: token_id.to_string(),
            }
            .into());
        }
        self.port.revoke_token(access_token, token_id).await?;
        Ok(())
    }
}
