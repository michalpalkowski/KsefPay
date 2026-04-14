use std::sync::Arc;

use crate::domain::auth::AccessToken;
use crate::domain::permission::{
    PermissionGrantRequest, PermissionRecord, PermissionRevokeRequest,
};
use crate::error::{DomainError, KSeFError};
use crate::ports::ksef_permissions::{KSeFPermissions, PermissionQueryRequest};

pub struct PermissionService {
    port: Arc<dyn KSeFPermissions>,
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionServiceError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error(transparent)]
    KSeF(#[from] KSeFError),
}

impl PermissionService {
    #[must_use]
    pub fn new(port: Arc<dyn KSeFPermissions>) -> Self {
        Self { port }
    }

    pub async fn grant(
        &self,
        access_token: &AccessToken,
        request: &PermissionGrantRequest,
    ) -> Result<(), PermissionServiceError> {
        request.validate()?;
        self.port.grant_permissions(access_token, request).await?;
        Ok(())
    }

    pub async fn revoke(
        &self,
        access_token: &AccessToken,
        request: &PermissionRevokeRequest,
    ) -> Result<(), PermissionServiceError> {
        request.validate()?;
        self.port.revoke_permissions(access_token, request).await?;
        Ok(())
    }

    pub async fn query(
        &self,
        access_token: &AccessToken,
        request: &PermissionQueryRequest,
    ) -> Result<Vec<PermissionRecord>, PermissionServiceError> {
        Ok(self.port.query_permissions(access_token, request).await?)
    }
}
