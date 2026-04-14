use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::domain::invoice::Direction;
use crate::error::DomainError;

/// `KSeF` query subject type — determines the role of our NIP in the queried invoices.
///
/// Protocol-first: the type system prevents passing a raw string where a subject type is expected.
/// Each variant maps to a `KSeF` API value and a logical invoice direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubjectType {
    /// Our NIP is the seller — invoices we issued.
    Subject1,
    /// Our NIP is the buyer — invoices received from suppliers.
    Subject2,
    /// Our NIP is a third party (e.g., intermediary with power of attorney).
    Subject3,
}

impl SubjectType {
    /// Value sent to the `KSeF` v2 API `subjectType` field (`PascalCase` per spec).
    #[must_use]
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Subject1 => "Subject1",
            Self::Subject2 => "Subject2",
            Self::Subject3 => "Subject3",
        }
    }

    /// Logical invoice direction from our perspective.
    #[must_use]
    pub fn to_direction(self) -> Direction {
        match self {
            Self::Subject1 => Direction::Outgoing,
            Self::Subject2 | Self::Subject3 => Direction::Incoming,
        }
    }
}

impl fmt::Display for SubjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Subject1 => write!(f, "subject1"),
            Self::Subject2 => write!(f, "subject2"),
            Self::Subject3 => write!(f, "subject3"),
        }
    }
}

impl FromStr for SubjectType {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "subject1" => Ok(Self::Subject1),
            "subject2" => Ok(Self::Subject2),
            "subject3" => Ok(Self::Subject3),
            other => Err(DomainError::InvalidParse {
                type_name: "SubjectType",
                value: other.to_string(),
            }),
        }
    }
}

/// Reference to an open interactive `KSeF` session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionReference(String);

impl SessionReference {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `KSeF`-assigned invoice number (e.g., `KSeF-1234567890-20260413-ABC123`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KSeFNumber(String);

impl KSeFNumber {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KSeFNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// UPO (Urzędowe Potwierdzenie Odbioru) — official receipt from `KSeF`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upo {
    pub reference: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpoVersion {
    V4_2,
    V4_3,
}

impl fmt::Display for UpoVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4_2 => write!(f, "4.2"),
            Self::V4_3 => write!(f, "4.3"),
        }
    }
}

impl FromStr for UpoVersion {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "4.2" => Ok(Self::V4_2),
            "4.3" => Ok(Self::V4_3),
            other => Err(DomainError::InvalidParse {
                type_name: "UpoVersion",
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpoPageResponse {
    pub version: UpoVersion,
    pub download_url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpoDownloadResult {
    pub upo: Upo,
    pub hash_sha256: Option<String>,
}

/// Metadata about an invoice returned from `KSeF` query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceMetadata {
    pub ksef_number: KSeFNumber,
    pub subject_nip: String,
    pub invoice_date: chrono::NaiveDate,
}

/// Criteria for fetching invoices from `KSeF`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceQuery {
    pub date_from: chrono::NaiveDate,
    pub date_to: chrono::NaiveDate,
    pub subject_type: SubjectType,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SubjectType ---

    #[test]
    fn subject_type_api_value_is_pascal_case() {
        assert_eq!(SubjectType::Subject1.api_value(), "Subject1");
        assert_eq!(SubjectType::Subject2.api_value(), "Subject2");
        assert_eq!(SubjectType::Subject3.api_value(), "Subject3");
    }

    #[test]
    fn subject_type_to_direction() {
        assert_eq!(SubjectType::Subject1.to_direction(), Direction::Outgoing);
        assert_eq!(SubjectType::Subject2.to_direction(), Direction::Incoming);
        assert_eq!(SubjectType::Subject3.to_direction(), Direction::Incoming);
    }

    #[test]
    fn subject_type_display_and_from_str_round_trip() {
        for st in [
            SubjectType::Subject1,
            SubjectType::Subject2,
            SubjectType::Subject3,
        ] {
            let s = st.to_string();
            let parsed: SubjectType = s.parse().unwrap();
            assert_eq!(st, parsed);
        }
    }

    #[test]
    fn subject_type_from_str_invalid_returns_error() {
        let err = "subject4".parse::<SubjectType>().unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidParse {
                type_name: "SubjectType",
                ..
            }
        ));
    }

    // --- SessionReference ---

    #[test]
    fn session_reference_encapsulated() {
        let r = SessionReference::new("ref-123".to_string());
        assert_eq!(r.as_str(), "ref-123");
        assert_eq!(r.to_string(), "ref-123");
    }

    // --- KSeFNumber ---

    #[test]
    fn ksef_number_encapsulated() {
        let n = KSeFNumber::new("KSeF-1234567890-20260413-ABC".to_string());
        assert_eq!(n.as_str(), "KSeF-1234567890-20260413-ABC");
        assert_eq!(n.to_string(), "KSeF-1234567890-20260413-ABC");
    }

    // --- UpoVersion ---

    #[test]
    fn upo_version_display_and_parse_roundtrip() {
        for version in [UpoVersion::V4_2, UpoVersion::V4_3] {
            let rendered = version.to_string();
            let parsed: UpoVersion = rendered.parse().unwrap();
            assert_eq!(version, parsed);
        }
    }

    #[test]
    fn upo_version_unknown_returns_error() {
        assert!(matches!(
            "5.0".parse::<UpoVersion>(),
            Err(DomainError::InvalidParse {
                type_name: "UpoVersion",
                ..
            })
        ));
    }
}
