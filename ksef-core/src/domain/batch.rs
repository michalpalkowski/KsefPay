use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

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
    pub upload_url: Option<UploadUrl>,
    pub part: BatchFilePartInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UploadUrl(String);

impl UploadUrl {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UploadUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for UploadUrl {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim();
        if value.is_empty() {
            return Err(DomainError::InvalidParse {
                type_name: "UploadUrl",
                value: s.to_string(),
            });
        }
        let Some((scheme, rest)) = value.split_once("://") else {
            return Err(DomainError::InvalidParse {
                type_name: "UploadUrl",
                value: s.to_string(),
            });
        };
        if rest.is_empty() {
            return Err(DomainError::InvalidParse {
                type_name: "UploadUrl",
                value: s.to_string(),
            });
        }
        if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
            return Err(DomainError::InvalidParse {
                type_name: "UploadUrl",
                value: s.to_string(),
            });
        }
        Ok(Self(value.to_string()))
    }
}

impl<'de> Deserialize<'de> for UploadUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
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

    #[test]
    fn upload_url_accepts_http_and_https() {
        assert_eq!(
            "https://upload.example/path?x=1"
                .parse::<UploadUrl>()
                .unwrap()
                .as_str(),
            "https://upload.example/path?x=1"
        );
        assert_eq!(
            "http://localhost:9000/part"
                .parse::<UploadUrl>()
                .unwrap()
                .as_str(),
            "http://localhost:9000/part"
        );
        assert_eq!(
            "HTTPS://UPLOAD.EXAMPLE/PATH"
                .parse::<UploadUrl>()
                .unwrap()
                .as_str(),
            "HTTPS://UPLOAD.EXAMPLE/PATH"
        );
    }

    #[test]
    fn upload_url_rejects_empty_or_non_http_scheme() {
        assert!("".parse::<UploadUrl>().is_err());
        assert!("   ".parse::<UploadUrl>().is_err());
        assert!("ftp://example.com/file".parse::<UploadUrl>().is_err());
        assert!("https://".parse::<UploadUrl>().is_err());
    }

    #[test]
    fn upload_url_deserialize_uses_same_validation() {
        let parsed: UploadUrl = serde_json::from_str("\"https://upload.example/part\"").unwrap();
        assert_eq!(parsed.as_str(), "https://upload.example/part");
        assert!(serde_json::from_str::<UploadUrl>("\"ftp://upload.example/part\"").is_err());
    }
}
