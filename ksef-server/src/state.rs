use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ksef_core::domain::nip::Nip;
use ksef_core::services::batch_service::BatchService;
use ksef_core::services::export_service::ExportService;
use ksef_core::services::fetch_service::FetchService;
use ksef_core::services::invoice_service::InvoiceService;
use ksef_core::services::offline_service::OfflineService;
use ksef_core::services::permission_service::PermissionService;
use ksef_core::services::qr_service::QRService;
use ksef_core::services::session_service::SessionService;

/// AES key + IV pair for export decryption.
pub type ExportKeyStore = Arc<Mutex<HashMap<String, (Vec<u8>, Vec<u8>)>>>;
use ksef_core::services::token_mgmt_service::TokenMgmtService;

/// Shared application state injected into Axum handlers.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub nip: Nip,
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
