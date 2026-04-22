use async_trait::async_trait;

use crate::domain::account_scope::AccountScope;
use crate::domain::invoice::{Direction, Invoice, InvoiceId, InvoiceStatus};
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;

/// Optional filter criteria for listing invoices.
///
/// Does **not** contain an `account_id` — the caller supplies an [`AccountScope`]
/// as a separate argument to [`InvoiceRepository::list`].  The scope is the sole
/// proof of authorisation; the filter is purely cosmetic (pagination, direction,
/// status).
#[derive(Debug, Clone, Default)]
pub struct InvoiceFilter {
    pub direction: Option<Direction>,
    pub status: Option<InvoiceStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl InvoiceFilter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        scope: &AccountScope,
    ) -> Result<Invoice, RepositoryError>;

    async fn update_status(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError>;

    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        ksef_number: &str,
    ) -> Result<(), RepositoryError>;

    async fn set_ksef_error(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
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
        scope: &AccountScope,
    ) -> Result<Option<Invoice>, RepositoryError>;

    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError>;

    async fn list(
        &self,
        scope: &AccountScope,
        filter: &InvoiceFilter,
    ) -> Result<Vec<Invoice>, RepositoryError>;
}
