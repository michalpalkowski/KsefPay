use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::export::ExportJob;
use crate::domain::session::InvoiceQuery;
use crate::error::KSeFError;

#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub query: InvoiceQuery,
}

/// Port: asynchronous invoice export.
#[async_trait]
pub trait KSeFExport: Send + Sync {
    async fn start_export(
        &self,
        access_token: &AccessToken,
        request: &ExportRequest,
    ) -> Result<ExportJob, KSeFError>;

    async fn get_export_status(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<ExportJob, KSeFError>;
}
