use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::peppol::PeppolProvider;
use crate::error::KSeFError;

#[derive(Debug, Clone)]
pub struct PeppolQueryRequest {
    pub page_offset: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone)]
pub struct PeppolProvidersResponse {
    pub items: Vec<PeppolProvider>,
    pub total: u32,
}

/// Port: `KSeF` PEPPOL provider registry.
#[async_trait]
pub trait KSeFPeppol: Send + Sync {
    async fn query_providers(
        &self,
        access_token: &AccessToken,
        request: &PeppolQueryRequest,
    ) -> Result<PeppolProvidersResponse, KSeFError>;
}
