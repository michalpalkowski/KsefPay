use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KsefCertificateType {
    Seal,
    Token,
    Offline,
}

impl fmt::Display for KsefCertificateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seal => write!(f, "seal"),
            Self::Token => write!(f, "token"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

impl FromStr for KsefCertificateType {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "seal" => Ok(Self::Seal),
            "token" => Ok(Self::Token),
            "offline" => Ok(Self::Offline),
            other => Err(DomainError::InvalidParse {
                type_name: "KsefCertificateType",
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnrollmentStatus {
    Pending,
    Approved,
    Rejected,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateEnrollment {
    pub reference_number: String,
    pub certificate_type: KsefCertificateType,
    pub status: EnrollmentStatus,
    pub submitted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateLimits {
    pub max_active: u32,
    pub active: u32,
    pub pending: u32,
}

impl CertificateLimits {
    #[must_use]
    pub fn can_enroll(&self) -> bool {
        self.active.saturating_add(self.pending) < self.max_active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_type_roundtrip_display_parse() {
        for ty in [
            KsefCertificateType::Seal,
            KsefCertificateType::Token,
            KsefCertificateType::Offline,
        ] {
            let parsed: KsefCertificateType = ty.to_string().parse().unwrap();
            assert_eq!(ty, parsed);
        }
    }

    #[test]
    fn certificate_limits_can_enroll_when_capacity_left() {
        let limits = CertificateLimits {
            max_active: 10,
            active: 4,
            pending: 3,
        };
        assert!(limits.can_enroll());
    }

    #[test]
    fn certificate_limits_cannot_enroll_when_at_capacity() {
        let limits = CertificateLimits {
            max_active: 5,
            active: 3,
            pending: 2,
        };
        assert!(!limits.can_enroll());
    }
}
