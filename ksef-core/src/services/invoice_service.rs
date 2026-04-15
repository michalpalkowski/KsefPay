use std::sync::Arc;

use chrono::NaiveDate;

use crate::domain::invoice::{
    Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType, LineItem, Money, Party,
    PaymentMethod,
};
use crate::domain::job::{Job, JobId, JobStatus};
use crate::domain::session::KSeFNumber;
use crate::error::{DomainError, QueueError, RepositoryError};
use crate::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use crate::ports::job_queue::JobQueue;
use crate::ports::transaction::AtomicScopeFactory;

/// Input data for creating a new invoice draft.
#[derive(Debug, Clone)]
pub struct CreateInvoiceInput {
    pub direction: Direction,
    pub invoice_type: InvoiceType,
    pub invoice_number: String,
    pub issue_date: NaiveDate,
    pub sale_date: NaiveDate,
    pub corrected_invoice_number: Option<String>,
    pub correction_reason: Option<String>,
    pub original_ksef_number: Option<KSeFNumber>,
    pub advance_payment_date: Option<NaiveDate>,
    pub seller: Party,
    pub buyer: Party,
    pub currency: Currency,
    pub line_items: Vec<LineItem>,
    pub payment_method: PaymentMethod,
    pub payment_deadline: NaiveDate,
    pub bank_account: Option<String>,
}

/// Application service orchestrating invoice operations.
pub struct InvoiceService {
    repo: Arc<dyn InvoiceRepository>,
    queue: Arc<dyn JobQueue>,
    atomic: Option<Arc<dyn AtomicScopeFactory>>,
}

#[derive(Debug, thiserror::Error)]
pub enum InvoiceServiceError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error(transparent)]
    Repository(#[from] RepositoryError),

    #[error(transparent)]
    Queue(#[from] QueueError),
}

impl InvoiceService {
    /// Create with separate repo + queue (for unit tests with mocks — no transactions).
    #[must_use]
    pub fn new(repo: Arc<dyn InvoiceRepository>, queue: Arc<dyn JobQueue>) -> Self {
        Self {
            repo,
            queue,
            atomic: None,
        }
    }

    /// Create with full transactional support (production).
    #[must_use]
    pub fn with_atomic(
        repo: Arc<dyn InvoiceRepository>,
        queue: Arc<dyn JobQueue>,
        atomic: Arc<dyn AtomicScopeFactory>,
    ) -> Self {
        Self {
            repo,
            queue,
            atomic: Some(atomic),
        }
    }

    /// Create a new invoice in `Draft` status.
    pub async fn create_draft(
        &self,
        input: CreateInvoiceInput,
    ) -> Result<Invoice, InvoiceServiceError> {
        let (total_net, total_vat, total_gross) = compute_totals(&input.line_items);

        let invoice = Invoice {
            id: InvoiceId::new(),
            direction: input.direction,
            status: InvoiceStatus::Draft,
            invoice_type: input.invoice_type,
            invoice_number: input.invoice_number,
            issue_date: input.issue_date,
            sale_date: Some(input.sale_date),
            corrected_invoice_number: input.corrected_invoice_number,
            correction_reason: input.correction_reason,
            original_ksef_number: input.original_ksef_number,
            advance_payment_date: input.advance_payment_date,
            seller: input.seller,
            buyer: input.buyer,
            currency: input.currency,
            line_items: input.line_items,
            total_net,
            total_vat,
            total_gross,
            payment_method: Some(input.payment_method),
            payment_deadline: Some(input.payment_deadline),
            bank_account: input.bank_account,
            ksef_number: None,
            ksef_error: None,
            raw_xml: None,
        };

        self.repo.save(&invoice).await?;
        Ok(invoice)
    }

    /// Submit a draft invoice for `KSeF` processing.
    ///
    /// Atomically transitions Draft -> Queued and enqueues a submission job.
    /// When `AtomicScopeFactory` is available, both operations happen in one
    /// database transaction. Without it (unit tests), they execute separately.
    pub async fn submit(&self, id: &InvoiceId) -> Result<(), InvoiceServiceError> {
        let invoice = self.repo.find_by_id(id).await?;
        let new_status = invoice.status.transition_to(InvoiceStatus::Queued)?;

        let job = Job {
            id: JobId::new(),
            job_type: "submit_invoice".to_string(),
            payload: serde_json::json!({ "invoice_id": id.to_string() }),
            status: JobStatus::Pending,
            attempts: 0,
            max_attempts: 3,
            last_error: None,
            created_at: chrono::Utc::now(),
        };

        if let Some(ref atomic) = self.atomic {
            // ACID: both operations in one transaction
            let tx = atomic.begin().await?;
            tx.update_status(id, new_status).await?;
            tx.enqueue(job).await?;
            tx.commit().await?;
        } else {
            // Non-transactional fallback (unit tests with mocks)
            self.repo.update_status(id, new_status).await?;
            self.queue.enqueue(job).await?;
        }

        Ok(())
    }

    /// Mark invoice as submitted to `KSeF` (worker picked it up).
    pub async fn mark_submitted(&self, id: &InvoiceId) -> Result<(), InvoiceServiceError> {
        let invoice = self.repo.find_by_id(id).await?;
        if invoice.status == InvoiceStatus::Submitted {
            // Idempotent for retried jobs that already moved out of `Queued`.
            return Ok(());
        }
        let new_status = invoice.status.transition_to(InvoiceStatus::Submitted)?;
        self.repo.update_status(id, new_status).await?;
        Ok(())
    }

    /// Mark invoice as accepted by `KSeF`, storing the assigned number.
    pub async fn mark_accepted(
        &self,
        id: &InvoiceId,
        ksef_number: &str,
    ) -> Result<(), InvoiceServiceError> {
        if let Some(ref atomic) = self.atomic {
            let tx = atomic.begin().await?;
            let invoice = tx.find_by_id(id).await?;
            let new_status = invoice.status.transition_to(InvoiceStatus::Accepted)?;
            tx.update_status(id, new_status).await?;
            tx.set_ksef_number(id, ksef_number).await?;
            tx.commit().await?;
            return Ok(());
        }

        let invoice = self.repo.find_by_id(id).await?;
        let new_status = invoice.status.transition_to(InvoiceStatus::Accepted)?;
        self.repo.update_status(id, new_status).await?;
        self.repo.set_ksef_number(id, ksef_number).await?;
        Ok(())
    }

    /// Mark invoice as rejected by `KSeF`, storing the error.
    pub async fn mark_rejected(
        &self,
        id: &InvoiceId,
        error: &str,
    ) -> Result<(), InvoiceServiceError> {
        if let Some(ref atomic) = self.atomic {
            let tx = atomic.begin().await?;
            let invoice = tx.find_by_id(id).await?;
            let new_status = invoice.status.transition_to(InvoiceStatus::Rejected)?;
            tx.update_status(id, new_status).await?;
            tx.set_ksef_error(id, error).await?;
            tx.commit().await?;
            return Ok(());
        }

        let invoice = self.repo.find_by_id(id).await?;
        let new_status = invoice.status.transition_to(InvoiceStatus::Rejected)?;
        self.repo.update_status(id, new_status).await?;
        self.repo.set_ksef_error(id, error).await?;
        Ok(())
    }

    /// Mark invoice as permanently failed.
    pub async fn mark_failed(
        &self,
        id: &InvoiceId,
        error: &str,
    ) -> Result<(), InvoiceServiceError> {
        if let Some(ref atomic) = self.atomic {
            let tx = atomic.begin().await?;
            let invoice = tx.find_by_id(id).await?;
            let new_status = invoice.status.transition_to(InvoiceStatus::Failed)?;
            tx.update_status(id, new_status).await?;
            tx.set_ksef_error(id, error).await?;
            tx.commit().await?;
            return Ok(());
        }

        let invoice = self.repo.find_by_id(id).await?;
        let new_status = invoice.status.transition_to(InvoiceStatus::Failed)?;
        self.repo.update_status(id, new_status).await?;
        self.repo.set_ksef_error(id, error).await?;
        Ok(())
    }

    pub async fn find(&self, id: &InvoiceId) -> Result<Invoice, InvoiceServiceError> {
        Ok(self.repo.find_by_id(id).await?)
    }

    pub async fn list(&self, filter: &InvoiceFilter) -> Result<Vec<Invoice>, InvoiceServiceError> {
        Ok(self.repo.list(filter).await?)
    }
}

fn compute_totals(items: &[LineItem]) -> (Money, Money, Money) {
    let total_net = items
        .iter()
        .fold(Money::from_grosze(0), |acc, item| acc + item.net_value);
    let total_vat = items
        .iter()
        .fold(Money::from_grosze(0), |acc, item| acc + item.vat_amount);
    let total_gross = items
        .iter()
        .fold(Money::from_grosze(0), |acc, item| acc + item.gross_value);
    (total_net, total_vat, total_gross)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixtures::sample_invoice;
    use crate::test_support::mock_invoice_repo::MockInvoiceRepo;
    use crate::test_support::mock_job_queue::MockJobQueue;

    fn make_service() -> (InvoiceService, Arc<MockInvoiceRepo>, Arc<MockJobQueue>) {
        let repo = Arc::new(MockInvoiceRepo::new());
        let queue = Arc::new(MockJobQueue::new());
        let service = InvoiceService::new(repo.clone(), queue.clone());
        (service, repo, queue)
    }

    fn make_input() -> CreateInvoiceInput {
        let inv = sample_invoice();
        CreateInvoiceInput {
            direction: inv.direction,
            invoice_type: inv.invoice_type,
            invoice_number: inv.invoice_number,
            issue_date: inv.issue_date,
            sale_date: inv.sale_date.unwrap(),
            corrected_invoice_number: inv.corrected_invoice_number,
            correction_reason: inv.correction_reason,
            original_ksef_number: inv.original_ksef_number,
            advance_payment_date: inv.advance_payment_date,
            seller: inv.seller,
            buyer: inv.buyer,
            currency: inv.currency,
            line_items: inv.line_items,
            payment_method: inv.payment_method.unwrap(),
            payment_deadline: inv.payment_deadline.unwrap(),
            bank_account: inv.bank_account,
        }
    }

    #[tokio::test]
    async fn create_draft_stores_invoice_with_draft_status() {
        let (service, repo, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();

        assert_eq!(invoice.status, InvoiceStatus::Draft);
        assert_eq!(repo.count(), 1);

        let found = service.find(&invoice.id).await.unwrap();
        assert_eq!(found.invoice_number, "FV/2026/04/001");
    }

    #[tokio::test]
    async fn create_draft_computes_totals_from_line_items() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();

        assert_eq!(invoice.total_net, Money::from_pln(24000, 0));
        assert_eq!(invoice.total_vat, Money::from_pln(5520, 0));
        assert_eq!(invoice.total_gross, Money::from_pln(29520, 0));
    }

    #[tokio::test]
    async fn create_draft_has_no_ksef_number() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();
        assert!(invoice.ksef_number.is_none());
        assert!(invoice.ksef_error.is_none());
    }

    #[tokio::test]
    async fn submit_transitions_draft_to_queued_and_enqueues_job() {
        let (service, _, queue) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();

        service.submit(&invoice.id).await.unwrap();

        let found = service.find(&invoice.id).await.unwrap();
        assert_eq!(found.status, InvoiceStatus::Queued);
        assert_eq!(queue.count(), 1);
    }

    #[tokio::test]
    async fn submit_already_queued_returns_error() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();
        service.submit(&invoice.id).await.unwrap();

        let err = service.submit(&invoice.id).await.unwrap_err();
        assert!(matches!(
            err,
            InvoiceServiceError::Domain(DomainError::InvalidStatusTransition { .. })
        ));
    }

    #[tokio::test]
    async fn submit_nonexistent_returns_not_found() {
        let (service, _, _) = make_service();
        let err = service.submit(&InvoiceId::new()).await.unwrap_err();
        assert!(matches!(
            err,
            InvoiceServiceError::Repository(RepositoryError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn mark_submitted_transitions_queued_to_submitted() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();
        service.submit(&invoice.id).await.unwrap();

        service.mark_submitted(&invoice.id).await.unwrap();

        let found = service.find(&invoice.id).await.unwrap();
        assert_eq!(found.status, InvoiceStatus::Submitted);
    }

    #[tokio::test]
    async fn mark_submitted_from_draft_returns_error() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();

        let err = service.mark_submitted(&invoice.id).await.unwrap_err();
        assert!(matches!(err, InvoiceServiceError::Domain(_)));
    }

    #[tokio::test]
    async fn mark_accepted_transitions_and_stores_ksef_number() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();
        service.submit(&invoice.id).await.unwrap();
        service.mark_submitted(&invoice.id).await.unwrap();

        service
            .mark_accepted(&invoice.id, "KSeF-2026-04-ABC")
            .await
            .unwrap();

        let found = service.find(&invoice.id).await.unwrap();
        assert_eq!(found.status, InvoiceStatus::Accepted);
        assert_eq!(found.ksef_number.unwrap().as_str(), "KSeF-2026-04-ABC");
    }

    #[tokio::test]
    async fn mark_rejected_transitions_and_stores_error() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();
        service.submit(&invoice.id).await.unwrap();
        service.mark_submitted(&invoice.id).await.unwrap();

        service
            .mark_rejected(&invoice.id, "XML schema validation failed")
            .await
            .unwrap();

        let found = service.find(&invoice.id).await.unwrap();
        assert_eq!(found.status, InvoiceStatus::Rejected);
        assert_eq!(
            found.ksef_error.as_deref(),
            Some("XML schema validation failed")
        );
    }

    #[tokio::test]
    async fn mark_failed_from_queued() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();
        service.submit(&invoice.id).await.unwrap();

        service
            .mark_failed(&invoice.id, "max retries exceeded")
            .await
            .unwrap();

        let found = service.find(&invoice.id).await.unwrap();
        assert_eq!(found.status, InvoiceStatus::Failed);
        assert_eq!(found.ksef_error.as_deref(), Some("max retries exceeded"));
    }

    #[tokio::test]
    async fn mark_failed_from_draft_returns_error() {
        let (service, _, _) = make_service();
        let invoice = service.create_draft(make_input()).await.unwrap();

        let err = service.mark_failed(&invoice.id, "err").await.unwrap_err();
        assert!(matches!(err, InvoiceServiceError::Domain(_)));
    }

    #[tokio::test]
    async fn list_returns_all_invoices() {
        let (service, _, _) = make_service();
        service.create_draft(make_input()).await.unwrap();
        service.create_draft(make_input()).await.unwrap();

        let nip = crate::domain::nip::Nip::parse("5260250274").unwrap();
        let all = service.list(&InvoiceFilter::for_account(nip)).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn full_happy_path_draft_to_accepted() {
        let (service, _, queue) = make_service();

        let invoice = service.create_draft(make_input()).await.unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Draft);

        service.submit(&invoice.id).await.unwrap();
        assert_eq!(queue.count(), 1);

        service.mark_submitted(&invoice.id).await.unwrap();

        service
            .mark_accepted(&invoice.id, "KSeF-2026-FINAL")
            .await
            .unwrap();

        let final_invoice = service.find(&invoice.id).await.unwrap();
        assert_eq!(final_invoice.status, InvoiceStatus::Accepted);
        assert_eq!(
            final_invoice.ksef_number.unwrap().as_str(),
            "KSeF-2026-FINAL"
        );
        assert!(final_invoice.ksef_error.is_none());
    }
}
