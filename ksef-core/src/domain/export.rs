use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportStatus {
    Pending,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportJob {
    pub reference_number: String,
    pub status: ExportStatus,
    pub download_url: Option<String>,
    pub error_message: Option<String>,
    /// Raw AES-256 key used to encrypt the export (32 bytes). Needed for decryption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key: Option<Vec<u8>>,
    /// AES IV used to encrypt the export (16 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_iv: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_export_must_have_download_url() {
        let job = ExportJob {
            reference_number: "exp-1".to_string(),
            status: ExportStatus::Completed,
            download_url: Some("https://example.com/export.zip".to_string()),
            error_message: None,
            encryption_key: None,
            encryption_iv: None,
        };
        assert!(job.download_url.is_some());
    }

    #[test]
    fn failed_export_may_have_error_message() {
        let job = ExportJob {
            reference_number: "exp-2".to_string(),
            status: ExportStatus::Failed,
            download_url: None,
            error_message: Some("processing failed".to_string()),
            encryption_key: None,
            encryption_iv: None,
        };
        assert!(job.error_message.is_some());
    }
}
