use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::domain::invoice::{InvoiceId, InvoiceStatus};
use crate::domain::job::Job;
use crate::domain::nip_account::NipAccountId;
use crate::error::KSeFError;
use crate::ports::encryption::InvoiceEncryptor;
use crate::ports::invoice_xml::InvoiceXmlConverter;
use crate::ports::invoice_xml_validator::InvoiceXmlValidator;
use crate::ports::job_queue::JobQueue;
use crate::ports::ksef_client::KSeFClient;
use crate::services::invoice_service::InvoiceService;
use crate::services::session_service::SessionService;

/// Handles execution of dequeued jobs by dispatching to the appropriate service.
pub struct JobWorker {
    queue: Arc<dyn JobQueue>,
    invoice_service: Arc<InvoiceService>,
    session_service: Arc<SessionService>,
    ksef_client: Arc<dyn KSeFClient>,
    encryptor: Arc<dyn InvoiceEncryptor>,
    xml_converter: Arc<dyn InvoiceXmlConverter>,
    xml_validator: Arc<dyn InvoiceXmlValidator>,
    poll_interval: Duration,
}

impl JobWorker {
    #[must_use]
    pub fn new(
        queue: Arc<dyn JobQueue>,
        invoice_service: Arc<InvoiceService>,
        session_service: Arc<SessionService>,
        ksef_client: Arc<dyn KSeFClient>,
        encryptor: Arc<dyn InvoiceEncryptor>,
        xml_converter: Arc<dyn InvoiceXmlConverter>,
        xml_validator: Arc<dyn InvoiceXmlValidator>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            queue,
            invoice_service,
            session_service,
            ksef_client,
            encryptor,
            xml_converter,
            xml_validator,
            poll_interval,
        }
    }

    /// Run the worker loop until the shutdown signal fires.
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) -> Result<(), String> {
        info!(
            "job worker starting, poll_interval={:?}",
            self.poll_interval
        );

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("job worker shutting down");
                        return Ok(());
                    }
                }
                () = tokio::time::sleep(self.poll_interval) => {
                    if let Err(err) = self.tick().await {
                        return Err(format!("worker tick failed: {err}"));
                    }
                }
            }
        }
    }

    /// Execute a single tick: dequeue one job and process it.
    pub async fn tick(&self) -> Result<(), String> {
        match self.queue.dequeue().await {
            Ok(Some(job)) => {
                info!(job_id = %job.id, job_type = %job.job_type, "processing job");
                self.process_job(&job).await?;
            }
            Ok(None) => {} // no jobs
            Err(e) => {
                return Err(format!("failed to dequeue job: {e}"));
            }
        }
        Ok(())
    }

    async fn process_job(&self, job: &Job) -> Result<(), String> {
        let result = match job.job_type.as_str() {
            "submit_invoice" => self.handle_submit_invoice(job).await,
            other => {
                warn!(job_type = other, "unknown job type");
                Err(format!("unknown job type: {other}"))
            }
        };

        match result {
            Ok(()) => {
                self.queue
                    .complete(&job.id)
                    .await
                    .map_err(|e| format!("failed to mark job {} complete: {e}", job.id))?;
            }
            Err(error) => {
                warn!(job_id = %job.id, "job failed: {error}");
                if job.attempts + 1 >= job.max_attempts {
                    let mut payload_error: Option<String> = None;
                    let ctx = match extract_job_context(job) {
                        Ok(ctx) => Some(ctx),
                        Err(err) => {
                            warn!(job_id = %job.id, "cannot extract invoice_id from payload: {err}");
                            payload_error = Some(err);
                            None
                        }
                    };

                    if let Some((invoice_id, account_id)) = ctx {
                        self.invoice_service
                            .mark_failed(&invoice_id, &account_id, &error)
                            .await
                            .map_err(|e| {
                                format!(
                                    "failed to mark invoice {} as failed for job {}: {e}",
                                    invoice_id, job.id
                                )
                            })?;
                    }

                    self.queue
                        .dead_letter(&job.id, &error)
                        .await
                        .map_err(|e| format!("failed to dead-letter job {}: {e}", job.id))?;

                    if let Some(payload_error) = payload_error {
                        return Err(format!(
                            "job {} reached max attempts but payload is invalid ({payload_error}); root error: {error}",
                            job.id
                        ));
                    }
                } else {
                    self.queue
                        .fail(&job.id, &error)
                        .await
                        .map_err(|e| format!("failed to mark job {} for retry: {e}", job.id))?;
                }
            }
        }
        Ok(())
    }

    async fn handle_submit_invoice(&self, job: &Job) -> Result<(), String> {
        let (invoice_id, account_id) = extract_job_context(job)?;

        let invoice = self
            .invoice_service
            .find(&invoice_id, &account_id)
            .await
            .map_err(|e| format!("failed to fetch invoice: {e}"))?;

        match invoice.status {
            InvoiceStatus::Queued => {
                self.invoice_service
                    .mark_submitted(&invoice_id, &account_id)
                    .await
                    .map_err(|e| format!("failed to mark submitted: {e}"))?;
            }
            InvoiceStatus::Submitted => {}
            other => {
                return Err(format!(
                    "invoice {invoice_id} has invalid status for submission: {other}"
                ));
            }
        }

        let xml = self
            .xml_converter
            .to_xml(&invoice)
            .map_err(|e| format!("failed to build FA(3) XML: {e}"))?;
        if let Err(err) = self.xml_validator.validate(&xml) {
            let rejection_reason =
                format!("XML schema validation failed before KSeF submission: {err}");
            self.invoice_service
                .mark_rejected(&invoice_id, &account_id, &rejection_reason)
                .await
                .map_err(|e| format!("failed to mark invoice rejected: {e}"))?;
            return Ok(());
        }
        let public_keys = self
            .ksef_client
            .fetch_public_keys()
            .await
            .map_err(|e| format!("failed to fetch KSeF public keys: {e}"))?;
        let key = public_keys
            .first()
            .ok_or_else(|| "KSeF returned zero public keys".to_string())?;

        let encrypted = self
            .encryptor
            .encrypt(&xml, key)
            .await
            .map_err(|e| format!("failed to encrypt invoice XML: {e}"))?;

        let seller_nip = invoice
            .seller
            .nip
            .as_ref()
            .ok_or_else(|| "seller NIP is required for submission".to_string())?;

        let session = self
            .session_service
            .ensure_session(seller_nip, &encrypted)
            .await
            .map_err(|e| format!("failed to ensure KSeF session: {e}"))?;
        let token_pair = self
            .session_service
            .ensure_token(seller_nip)
            .await
            .map_err(|e| format!("failed to ensure KSeF access token: {e}"))?;

        let ksef_number = match self
            .ksef_client
            .send_invoice(&token_pair.access_token, &session, &encrypted)
            .await
        {
            Ok(number) => number,
            Err(err) => {
                if let Some(code) = terminal_rejection_code(&err) {
                    let rejection_reason = format!("KSeF invoice status {code}: {err}");
                    self.invoice_service
                        .mark_rejected(&invoice_id, &account_id, &rejection_reason)
                        .await
                        .map_err(|e| format!("failed to mark invoice rejected: {e}"))?;

                    if let Err(close_err) = self.session_service.close_session(seller_nip).await {
                        warn!(
                            invoice_id = %invoice_id,
                            session = %session,
                            error = %close_err,
                            "failed to close KSeF session after rejected invoice"
                        );
                    }

                    return Ok(());
                }

                return Err(format!("failed to send invoice to KSeF: {err}"));
            }
        };

        info!(
            invoice_id = %invoice_id,
            ksef_number = %ksef_number,
            "invoice accepted by KSeF"
        );

        self.invoice_service
            .mark_accepted(&invoice_id, &account_id, ksef_number.as_str())
            .await
            .map_err(|e| format!("failed to mark invoice accepted: {e}"))?;

        // Finalize interactive session immediately so invoice becomes visible
        // for downstream queries without waiting for another submission cycle.
        if let Err(err) = self.session_service.close_session(seller_nip).await {
            warn!(
                invoice_id = %invoice_id,
                session = %session,
                error = %err,
                "failed to close KSeF session after accepted invoice"
            );
        }

        Ok(())
    }
}

fn terminal_rejection_code(err: &KSeFError) -> Option<i64> {
    let KSeFError::InvoiceSubmissionFailed(message) = err else {
        return None;
    };
    let code = message
        .strip_prefix("invoice status code ")?
        .split(':')
        .next()?
        .trim()
        .parse::<i64>()
        .ok()?;

    match code {
        410 | 430 | 435 | 450 => Some(code),
        _ => None,
    }
}

fn extract_job_context(job: &Job) -> Result<(InvoiceId, NipAccountId), String> {
    let raw_invoice_id = job
        .payload
        .get("invoice_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing invoice_id in payload".to_string())?;
    let invoice_id = raw_invoice_id
        .parse::<InvoiceId>()
        .map_err(|_| format!("invalid invoice_id in payload: {raw_invoice_id}"))?;

    let raw_account_id = job
        .payload
        .get("nip_account_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing nip_account_id in payload".to_string())?;
    let account_id = raw_account_id
        .parse::<NipAccountId>()
        .map_err(|_| format!("invalid nip_account_id in payload: {raw_account_id}"))?;

    Ok((invoice_id, account_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::environment::KSeFEnvironment;
    use crate::domain::job::JobStatus;
    use crate::domain::nip_account::NipAccountId;
    use crate::infra::fa3::{Fa3XmlConverter, Fa3XsdValidator};
    use crate::services::invoice_service::{CreateInvoiceInput, InvoiceService};
    use crate::services::session_service::SessionService;
    use crate::test_support::fixtures::sample_invoice;
    use crate::test_support::mock_invoice_repo::MockInvoiceRepo;
    use crate::test_support::mock_job_queue::MockJobQueue;
    use crate::test_support::mock_ksef::{
        MockEncryptor, MockKSeFAuth, MockKSeFClient, MockXadesSigner,
    };
    use crate::test_support::mock_session_repo::MockSessionRepo;

    struct RejectingXmlValidator;

    impl InvoiceXmlValidator for RejectingXmlValidator {
        fn validate(
            &self,
            _xml: &crate::domain::xml::InvoiceXml,
        ) -> Result<(), crate::error::XmlError> {
            Err(crate::error::XmlError::ValidationFailed(
                "forced validation failure".to_string(),
            ))
        }
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

    fn test_account_id() -> NipAccountId {
        NipAccountId::from_uuid(uuid::Uuid::from_u128(1))
    }

    fn make_worker() -> (JobWorker, Arc<MockJobQueue>, Arc<InvoiceService>) {
        let repo = Arc::new(MockInvoiceRepo::new());
        let queue = Arc::new(MockJobQueue::new());
        let invoice_service = Arc::new(InvoiceService::new(repo.clone(), queue.clone()));
        let auth = Arc::new(MockKSeFAuth::new());
        let signer = Arc::new(MockXadesSigner);
        let client = Arc::new(MockKSeFClient::new());
        let session_repo = Arc::new(MockSessionRepo::new());
        let session_service = Arc::new(SessionService::new(
            auth,
            signer,
            client.clone(),
            session_repo,
            KSeFEnvironment::Test,
        ));
        let encryptor = Arc::new(MockEncryptor);
        let xml_converter = Arc::new(Fa3XmlConverter);
        let xml_validator = Arc::new(Fa3XsdValidator::new());
        let worker = JobWorker::new(
            queue.clone(),
            invoice_service.clone(),
            session_service,
            client,
            encryptor,
            xml_converter,
            xml_validator,
            Duration::from_millis(10),
        );
        (worker, queue, invoice_service)
    }

    // --- tick ---

    #[tokio::test]
    async fn tick_processes_submit_invoice_job() {
        let (worker, queue, invoice_service) = make_worker();

        // Create and submit an invoice (Draft -> Queued, enqueues job)
        let invoice = invoice_service
            .create_draft(make_input(), test_account_id())
            .await
            .unwrap();
        invoice_service
            .submit(&invoice.id, &test_account_id())
            .await
            .unwrap();

        // Worker tick processes the job
        worker.tick().await.unwrap();

        // Invoice should be Accepted now (full submit flow).
        let found = invoice_service
            .find(&invoice.id, &test_account_id())
            .await
            .unwrap();
        assert_eq!(
            found.status,
            crate::domain::invoice::InvoiceStatus::Accepted
        );
        assert_eq!(found.ksef_number.unwrap().as_str(), "KSeF-MOCK-1");

        // Job should be completed (not pending or dead-lettered)
        assert!(queue.list_pending().await.unwrap().is_empty());
        assert!(queue.list_dead_letter().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tick_on_empty_queue_does_nothing() {
        let (worker, _, _) = make_worker();
        worker.tick().await.unwrap(); // should not panic
    }

    #[tokio::test]
    async fn tick_unknown_job_type_requeues_job() {
        let (worker, queue, _) = make_worker();

        let job = crate::domain::job::Job {
            id: crate::domain::job::JobId::new(),
            job_type: "unknown_type".to_string(),
            payload: serde_json::json!({}),
            status: JobStatus::Pending,
            attempts: 0,
            max_attempts: 3,
            last_error: None,
            created_at: chrono::Utc::now(),
        };
        queue.enqueue(job).await.unwrap();

        worker.tick().await.unwrap();

        // Job should be queued for retry (pending), not completed/dead-lettered.
        let pending = queue.list_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        let jobs = queue.snapshot();
        let job = &jobs[0];
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.attempts, 1);
        assert!(
            job.last_error
                .as_ref()
                .unwrap()
                .contains("unknown job type")
        );
    }

    #[tokio::test]
    async fn tick_bad_payload_requeues_job() {
        let (worker, queue, _) = make_worker();

        let job = crate::domain::job::Job {
            id: crate::domain::job::JobId::new(),
            job_type: "submit_invoice".to_string(),
            payload: serde_json::json!({}), // missing invoice_id
            status: JobStatus::Pending,
            attempts: 0,
            max_attempts: 3,
            last_error: None,
            created_at: chrono::Utc::now(),
        };
        queue.enqueue(job).await.unwrap();

        worker.tick().await.unwrap();

        let jobs = queue.snapshot();
        assert_eq!(jobs[0].status, JobStatus::Pending);
        assert_eq!(jobs[0].attempts, 1);
        assert!(
            jobs[0]
                .last_error
                .as_ref()
                .unwrap()
                .contains("missing invoice_id")
        );
    }

    // --- shutdown ---

    #[tokio::test]
    async fn run_stops_on_shutdown_signal() {
        let (worker, _, _) = make_worker();
        let (tx, rx) = watch::channel(false);

        let handle = tokio::spawn(async move { worker.run(rx).await });

        // Signal shutdown
        tx.send(true).unwrap();

        // Worker should stop within a reasonable time
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("worker should stop within 2 seconds")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn tick_terminal_ksef_rejection_marks_invoice_rejected() {
        let repo = Arc::new(MockInvoiceRepo::new());
        let queue = Arc::new(MockJobQueue::new());
        let invoice_service = Arc::new(InvoiceService::new(repo.clone(), queue.clone()));
        let auth = Arc::new(MockKSeFAuth::new());
        let signer = Arc::new(MockXadesSigner);
        let client = Arc::new(MockKSeFClient::new());
        client.set_send_errors(vec![KSeFError::InvoiceSubmissionFailed(
            "invoice status code 450: Błąd weryfikacji semantyki dokumentu faktury".to_string(),
        )]);
        let session_repo = Arc::new(MockSessionRepo::new());
        let session_service = Arc::new(SessionService::new(
            auth,
            signer,
            client.clone(),
            session_repo,
            KSeFEnvironment::Test,
        ));
        let encryptor = Arc::new(MockEncryptor);
        let xml_converter = Arc::new(Fa3XmlConverter);
        let xml_validator = Arc::new(Fa3XsdValidator::new());
        let worker = JobWorker::new(
            queue.clone(),
            invoice_service.clone(),
            session_service,
            client,
            encryptor,
            xml_converter,
            xml_validator,
            Duration::from_millis(10),
        );

        let invoice = invoice_service
            .create_draft(make_input(), test_account_id())
            .await
            .unwrap();
        invoice_service
            .submit(&invoice.id, &test_account_id())
            .await
            .unwrap();

        worker.tick().await.unwrap();

        let found = invoice_service
            .find(&invoice.id, &test_account_id())
            .await
            .unwrap();
        assert_eq!(
            found.status,
            crate::domain::invoice::InvoiceStatus::Rejected
        );
        assert!(
            found
                .ksef_error
                .as_deref()
                .unwrap_or_default()
                .contains("450")
        );

        assert!(queue.list_pending().await.unwrap().is_empty());
        assert!(queue.list_dead_letter().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tick_preflight_xsd_failure_marks_invoice_rejected_without_ksef_call() {
        let repo = Arc::new(MockInvoiceRepo::new());
        let queue = Arc::new(MockJobQueue::new());
        let invoice_service = Arc::new(InvoiceService::new(repo.clone(), queue.clone()));
        let auth = Arc::new(MockKSeFAuth::new());
        let signer = Arc::new(MockXadesSigner);
        let client = Arc::new(MockKSeFClient::new());
        let session_repo = Arc::new(MockSessionRepo::new());
        let session_service = Arc::new(SessionService::new(
            auth,
            signer,
            client.clone(),
            session_repo,
            KSeFEnvironment::Test,
        ));
        let encryptor = Arc::new(MockEncryptor);
        let xml_converter = Arc::new(Fa3XmlConverter);
        let xml_validator = Arc::new(RejectingXmlValidator);
        let worker = JobWorker::new(
            queue.clone(),
            invoice_service.clone(),
            session_service,
            client,
            encryptor,
            xml_converter,
            xml_validator,
            Duration::from_millis(10),
        );

        let invoice = invoice_service
            .create_draft(make_input(), test_account_id())
            .await
            .unwrap();
        invoice_service
            .submit(&invoice.id, &test_account_id())
            .await
            .unwrap();

        worker.tick().await.unwrap();

        let found = invoice_service
            .find(&invoice.id, &test_account_id())
            .await
            .unwrap();
        assert_eq!(
            found.status,
            crate::domain::invoice::InvoiceStatus::Rejected
        );
        assert!(
            found
                .ksef_error
                .as_deref()
                .unwrap_or_default()
                .contains("XML schema validation failed before KSeF submission")
        );

        assert!(queue.list_pending().await.unwrap().is_empty());
        assert!(queue.list_dead_letter().await.unwrap().is_empty());
    }
}
