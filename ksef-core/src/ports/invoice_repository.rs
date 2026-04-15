use async_trait::async_trait;

use crate::domain::invoice::{Direction, Invoice, InvoiceId, InvoiceStatus};
use crate::domain::nip::Nip;
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;

/// Filter criteria for listing invoices.
///
/// `account_nip` is required — the type system enforces tenant isolation.
/// Every query must specify which NIP's invoices to return.
#[derive(Debug, Clone)]
pub struct InvoiceFilter {
    /// Tenant boundary: only invoices where this NIP is a party (seller or buyer).
    pub account_nip: Nip,
    pub direction: Option<Direction>,
    pub status: Option<InvoiceStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl InvoiceFilter {
    /// Create a filter scoped to the given NIP account.
    #[must_use]
    pub fn for_account(nip: Nip) -> Self {
        Self {
            account_nip: nip,
            direction: None,
            status: None,
            limit: None,
            offset: None,
        }
    }

    #[must_use]
    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: InvoiceStatus) -> Self {
        self.status = Some(status);
        self
    }
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
