use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::nip_account::NipAccountId;
use ksef_core::ports::invoice_sequence::InvoiceSequenceRepository;
use ksef_core::ports::local_token_repository::LocalTokenRepository;
use ksef_core::ports::nip_account_repository::NipAccountRepository;
use ksef_core::ports::user_repository::UserRepository;
use ksef_core::services::audit_service::AuditService;
use ksef_core::services::batch_service::BatchService;
use ksef_core::services::company_lookup_service::CompanyLookupService;
use ksef_core::services::export_service::ExportService;
use ksef_core::services::fetch_service::FetchService;
use ksef_core::services::invoice_service::InvoiceService;
use ksef_core::services::offline_service::OfflineService;
use ksef_core::services::permission_service::PermissionService;
use ksef_core::services::qr_service::QRService;
use ksef_core::services::session_service::SessionService;
use ksef_core::services::token_mgmt_service::TokenMgmtService;

use crate::auth_rate_limit::AuthRateLimiter;

/// AES key + IV pair for export decryption.
pub type ExportKeyStore = Arc<Mutex<HashMap<(NipAccountId, String), (Vec<u8>, Vec<u8>)>>>;

/// Status of a background fetch job.
#[derive(Clone)]
pub enum FetchJobStatus {
    Running,
    Done {
        inserted: u32,
        updated: u32,
        errors: Vec<String>,
    },
    Failed(String),
}

/// In-memory store for background fetch jobs, keyed by NIP account ID.
pub type FetchJobStore = Arc<Mutex<HashMap<NipAccountId, FetchJobStatus>>>;

/// Shared application state injected into Axum handlers.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub ksef_environment: KSeFEnvironment,
    pub user_repo: Arc<dyn UserRepository>,
    pub nip_account_repo: Arc<dyn NipAccountRepository>,
    pub company_lookup_service: Arc<CompanyLookupService>,
    pub invoice_sequence: Arc<dyn InvoiceSequenceRepository>,
    pub invoice_service: Arc<InvoiceService>,
    pub fetch_service: Arc<FetchService>,
    pub session_service: Arc<SessionService>,
    pub batch_service: Arc<BatchService>,
    pub permission_service: Arc<PermissionService>,
    pub token_mgmt_service: Arc<TokenMgmtService>,
    pub local_token_repo: Arc<dyn LocalTokenRepository>,
    pub export_service: Arc<ExportService>,
    pub offline_service: Arc<OfflineService>,
    pub qr_service: Arc<QRService>,
    pub audit_service: Arc<AuditService>,
    /// Temporary store for export encryption keys keyed by `(account_id, reference)`.
    pub export_keys: ExportKeyStore,
    /// Background fetch job statuses keyed by NIP account ID.
    pub fetch_jobs: FetchJobStore,
    /// Rate limiter for auth endpoints (`/login`, `/register`).
    pub auth_rate_limiter: AuthRateLimiter,
}
