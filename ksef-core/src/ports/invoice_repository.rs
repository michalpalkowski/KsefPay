use async_trait::async_trait;

use crate::domain::invoice::{Direction, Invoice, InvoiceId, InvoiceStatus};
use crate::domain::nip_account::NipAccountId;
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;

/// Filter criteria for listing invoices.
///
/// `account_id` is required — the type system enforces tenant isolation.
/// Every query must specify which account's invoices to return.
#[derive(Debug, Clone)]
pub struct InvoiceFilter {
    /// Tenant boundary: only invoices belonging to this NIP account.
    pub account_id: NipAccountId,
    pub direction: Option<Direction>,
    pub status: Option<InvoiceStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl InvoiceFilter {
    /// Create a filter scoped to the given NIP account.
    #[must_use]
    pub fn for_account(account_id: NipAccountId) -> Self {
        Self {
            account_id,
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

    async fn find_by_id(
        &self,
        id: &InvoiceId,
        account_id: &NipAccountId,
    ) -> Result<Invoice, RepositoryError>;

    async fn update_status(
        &self,
        id: &InvoiceId,
        account_id: &NipAccountId,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError>;

    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        account_id: &NipAccountId,
        ksef_number: &str,
    ) -> Result<(), RepositoryError>;

    async fn set_ksef_error(
        &self,
        id: &InvoiceId,
        account_id: &NipAccountId,
        error: &str,
    ) -> Result<(), RepositoryError>;

    /// Find an invoice by its KSeF-assigned number across all accounts.
    ///
    /// # Warning
    ///
    /// This method does **not** filter by account and may return invoices from any tenant.
    /// Prefer [`InvoiceRepository::find_by_ksef_number_and_account`] in all production code.
    /// This variant is retained only for internal background-job contexts that operate
    /// before the account is fully resolved.
    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError>;

    async fn find_by_ksef_number_and_account(
        &self,
        ksef_number: &KSeFNumber,
        account_id: &NipAccountId,
    ) -> Result<Option<Invoice>, RepositoryError>;

    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError>;

    async fn list(&self, filter: &InvoiceFilter) -> Result<Vec<Invoice>, RepositoryError>;
}
