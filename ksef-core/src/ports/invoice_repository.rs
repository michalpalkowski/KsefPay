use async_trait::async_trait;

use crate::domain::invoice::{Direction, Invoice, InvoiceId, InvoiceStatus};
use crate::domain::nip::Nip;
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;

/// Filter criteria for listing invoices.
#[derive(Debug, Default, Clone)]
pub struct InvoiceFilter {
    pub direction: Option<Direction>,
    pub status: Option<InvoiceStatus>,
    pub nip_seller: Option<Nip>,
    pub nip_buyer: Option<Nip>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Port: invoice persistence.
#[async_trait]
pub trait InvoiceRepository: Send + Sync {
    async fn save(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError>;

    async fn find_by_id(&self, id: &InvoiceId) -> Result<Invoice, RepositoryError>;

    async fn update_status(
        &self,
        id: &InvoiceId,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError>;

    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        ksef_number: &str,
    ) -> Result<(), RepositoryError>;

    async fn set_ksef_error(&self, id: &InvoiceId, error: &str) -> Result<(), RepositoryError>;

    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError>;

    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError>;

    async fn list(&self, filter: &InvoiceFilter) -> Result<Vec<Invoice>, RepositoryError>;
}
