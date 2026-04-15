use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::ports::invoice_sequence::InvoiceSequenceRepository;
use ksef_core::ports::nip_account_repository::NipAccountRepository;
use ksef_core::ports::user_repository::UserRepository;
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

/// AES key + IV pair for export decryption.
pub type ExportKeyStore = Arc<Mutex<HashMap<String, (Vec<u8>, Vec<u8>)>>>;

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
    pub export_service: Arc<ExportService>,
    pub offline_service: Arc<OfflineService>,
    pub qr_service: Arc<QRService>,
    /// Temporary store for export encryption keys keyed by reference number.
    pub export_keys: ExportKeyStore,
}
