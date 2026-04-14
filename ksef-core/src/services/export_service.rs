use std::sync::Arc;
use std::time::Duration;

use crate::domain::auth::AccessToken;
use crate::domain::export::{ExportJob, ExportStatus};
use crate::domain::session::InvoiceQuery;
use crate::error::{CryptoError, KSeFError};
use crate::ports::invoice_decryptor::InvoiceDecryptor;
use crate::ports::ksef_export::{ExportRequest, KSeFExport};

pub struct ExportService {
    port: Arc<dyn KSeFExport>,
    decryptor: Arc<dyn InvoiceDecryptor>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportServiceError {
    #[error(transparent)]
    KSeF(#[from] KSeFError),

    #[error(transparent)]
    Crypto(#[from] CryptoError),

    #[error("export failed: {0}")]
    ExportFailed(String),

    #[error("export polling timed out after {0} attempts")]
    PollTimeout(usize),

    #[error("download failed: {0}")]
    DownloadFailed(String),
}

impl ExportService {
    #[must_use]
    pub fn new(port: Arc<dyn KSeFExport>, decryptor: Arc<dyn InvoiceDecryptor>) -> Self {
        Self { port, decryptor }
    }

    pub async fn start(
        &self,
        access_token: &AccessToken,
        query: InvoiceQuery,
    ) -> Result<ExportJob, ExportServiceError> {
        let request = ExportRequest { query };
        Ok(self.port.start_export(access_token, &request).await?)
    }

    pub async fn get_status(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<ExportJob, ExportServiceError> {
        if reference_number.trim().is_empty() {
            return Err(ExportServiceError::ExportFailed(
                "reference number cannot be empty".to_string(),
            ));
        }
        Ok(self
            .port
            .get_export_status(access_token, reference_number)
            .await?)
    }

    pub async fn wait_until_complete(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
        max_attempts: usize,
        delay: Duration,
    ) -> Result<ExportJob, ExportServiceError> {
        for attempt in 1..=max_attempts {
            let status = self.get_status(access_token, reference_number).await?;
            match status.status {
                ExportStatus::Completed => return Ok(status),
                ExportStatus::Failed => {
                    return Err(ExportServiceError::ExportFailed(
                        status
                            .error_message
                            .unwrap_or_else(|| "export failed without error details".to_string()),
                    ));
                }
                ExportStatus::Pending => {
                    if attempt < max_attempts {
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
        Err(ExportServiceError::PollTimeout(max_attempts))
    }

    /// Download an encrypted export file and decrypt it with the stored AES key.
    /// Returns the decrypted ZIP bytes.
    pub async fn download_and_decrypt(
        &self,
        download_url: &str,
        encryption_key: &[u8],
        encryption_iv: &[u8],
    ) -> Result<Vec<u8>, ExportServiceError> {
        let response = reqwest::get(download_url)
            .await
            .map_err(|e| ExportServiceError::DownloadFailed(format!("HTTP request failed: {e}")))?;

        if !response.status().is_success() {
            return Err(ExportServiceError::DownloadFailed(format!(
                "HTTP {}: {}",
                response.status(),
                response.status().canonical_reason().unwrap_or("error"),
            )));
        }

        let encrypted_bytes = response.bytes().await.map_err(|e| {
            ExportServiceError::DownloadFailed(format!("reading response body failed: {e}"))
        })?;

        let decrypted = self
            .decryptor
            .decrypt(&encrypted_bytes, encryption_key, encryption_iv)?;
        Ok(decrypted)
    }
}
