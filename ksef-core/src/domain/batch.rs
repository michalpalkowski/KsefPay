use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchFileInfo {
    pub file_name: String,
    pub file_size_bytes: u64,
    pub file_hash_sha256_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchFilePartInfo {
    pub part_number: u32,
    pub offset_bytes: u64,
    pub size_bytes: u64,
    pub hash_sha256_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchArchive {
    pub zip_bytes: Vec<u8>,
    pub file_info: BatchFileInfo,
    pub parts: Vec<BatchFilePartInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartUploadRequest {
    pub session_reference: String,
    pub upload_url: String,
    pub part: BatchFilePartInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchSessionStatus {
    Created,
    Uploading,
    Uploaded,
    Processing,
    Completed,
    Failed,
    Closed,
}

impl BatchSessionStatus {
    pub fn transition_to(self, target: Self) -> Result<Self, DomainError> {
        let valid = matches!(
            (self, target),
            (Self::Created, Self::Uploading)
                | (Self::Uploading, Self::Uploaded | Self::Failed)
                | (Self::Uploaded, Self::Processing | Self::Closed)
                | (Self::Processing, Self::Completed | Self::Failed)
                | (Self::Completed | Self::Failed, Self::Closed)
        );

        if valid {
            Ok(target)
        } else {
            Err(DomainError::InvalidStatusTransition {
                from: self.to_string(),
                to: target.to_string(),
            })
        }
    }
}

impl fmt::Display for BatchSessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Uploading => write!(f, "uploading"),
            Self::Uploaded => write!(f, "uploaded"),
            Self::Processing => write!(f, "processing"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchSession {
    pub reference_number: String,
    pub status: BatchSessionStatus,
    pub created_at: DateTime<Utc>,
    pub files: Vec<BatchFileInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_status_happy_path_transitions_are_valid() {
        let s1 = BatchSessionStatus::Created
            .transition_to(BatchSessionStatus::Uploading)
            .unwrap();
        let s2 = s1.transition_to(BatchSessionStatus::Uploaded).unwrap();
        let s3 = s2.transition_to(BatchSessionStatus::Processing).unwrap();
        let s4 = s3.transition_to(BatchSessionStatus::Completed).unwrap();
        let s5 = s4.transition_to(BatchSessionStatus::Closed).unwrap();
        assert_eq!(s5, BatchSessionStatus::Closed);
    }

    #[test]
    fn batch_status_invalid_transition_returns_error() {
        assert!(matches!(
            BatchSessionStatus::Created.transition_to(BatchSessionStatus::Completed),
            Err(DomainError::InvalidStatusTransition { .. })
        ));
    }
}
