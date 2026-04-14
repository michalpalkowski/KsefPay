use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::nip::Nip;
use crate::domain::permission::{
    PermissionGrantRequest, PermissionRecord, PermissionRevokeRequest, PermissionType,
};
use crate::error::KSeFError;

#[derive(Debug, Clone)]
pub struct PermissionQueryRequest {
    pub context_nip: Nip,
    pub authorized_nip: Option<Nip>,
    pub permission: Option<PermissionType>,
}

/// Port: `KSeF` permissions management.
#[async_trait]
pub trait KSeFPermissions: Send + Sync {
    async fn grant_permissions(
        &self,
        access_token: &AccessToken,
        request: &PermissionGrantRequest,
    ) -> Result<(), KSeFError>;

    async fn revoke_permissions(
        &self,
        access_token: &AccessToken,
        request: &PermissionRevokeRequest,
    ) -> Result<(), KSeFError>;

    async fn query_permissions(
        &self,
        access_token: &AccessToken,
        request: &PermissionQueryRequest,
    ) -> Result<Vec<PermissionRecord>, KSeFError>;
}
