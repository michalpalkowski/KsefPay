use async_trait::async_trait;
use sqlx::PgPool;

use crate::domain::invoice::{Invoice, InvoiceId, InvoiceStatus};
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;
use crate::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};

use super::queries;

pub struct PgInvoiceRepo {
    pool: PgPool,
}

impl PgInvoiceRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InvoiceRepository for PgInvoiceRepo {
    async fn save(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        queries::invoice::save(&self.pool, invoice).await
    }

    async fn find_by_id(&self, id: &InvoiceId) -> Result<Invoice, RepositoryError> {
        queries::invoice::find_by_id(&self.pool, id).await
    }

    async fn update_status(
        &self,
        id: &InvoiceId,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError> {
        queries::invoice::update_status(&self.pool, id, status).await
    }

    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        ksef_number: &str,
    ) -> Result<(), RepositoryError> {
        queries::invoice::set_ksef_number(&self.pool, id, ksef_number).await
    }

    async fn set_ksef_error(&self, id: &InvoiceId, error: &str) -> Result<(), RepositoryError> {
        queries::invoice::set_ksef_error(&self.pool, id, error).await
    }

    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError> {
        queries::invoice::find_by_ksef_number(&self.pool, ksef_number).await
    }

    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        queries::invoice::upsert_by_ksef_number(&self.pool, invoice).await
    }

    async fn list(&self, filter: &InvoiceFilter) -> Result<Vec<Invoice>, RepositoryError> {
        queries::invoice::list(&self.pool, filter).await
    }
}
