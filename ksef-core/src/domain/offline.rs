use std::fmt;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::invoice::InvoiceId;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OfflineMode {
    Offline24,
    Offline,
    Awaryjny,
}

impl OfflineMode {
    #[must_use]
    pub fn deadline(
        self,
        created_at: DateTime<Utc>,
        offline_hours: i64,
        awaryjny_hours: i64,
    ) -> DateTime<Utc> {
        match self {
            Self::Offline24 => created_at + Duration::hours(24),
            Self::Offline => created_at + Duration::hours(offline_hours),
            Self::Awaryjny => created_at + Duration::hours(awaryjny_hours),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OfflineInvoiceStatus {
    Generated,
    Queued,
    Submitted,
    Accepted,
    Rejected,
    Expired,
}

impl OfflineInvoiceStatus {
    pub fn transition_to(self, target: Self) -> Result<Self, DomainError> {
        let valid = matches!(
            (self, target),
            (Self::Generated, Self::Queued | Self::Expired)
                | (Self::Queued, Self::Submitted | Self::Expired)
                | (
                    Self::Submitted,
                    Self::Accepted | Self::Rejected | Self::Expired
                )
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

impl fmt::Display for OfflineInvoiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generated => write!(f, "generated"),
            Self::Queued => write!(f, "queued"),
            Self::Submitted => write!(f, "submitted"),
            Self::Accepted => write!(f, "accepted"),
            Self::Rejected => write!(f, "rejected"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineInvoice {
    pub invoice_id: InvoiceId,
    pub mode: OfflineMode,
    pub status: OfflineInvoiceStatus,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

impl OfflineInvoice {
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now > self.deadline_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline24_deadline_is_exactly_24h() {
        let created_at = DateTime::parse_from_rfc3339("2026-04-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let deadline = OfflineMode::Offline24.deadline(created_at, 48, 72);
        assert_eq!(deadline, created_at + Duration::hours(24));
    }

    #[test]
    fn offline_status_happy_path_transitions_are_valid() {
        let s1 = OfflineInvoiceStatus::Generated
            .transition_to(OfflineInvoiceStatus::Queued)
            .unwrap();
        let s2 = s1.transition_to(OfflineInvoiceStatus::Submitted).unwrap();
        let s3 = s2.transition_to(OfflineInvoiceStatus::Accepted).unwrap();
        assert_eq!(s3, OfflineInvoiceStatus::Accepted);
    }

    #[test]
    fn offline_status_invalid_transition_returns_error() {
        assert!(matches!(
            OfflineInvoiceStatus::Generated.transition_to(OfflineInvoiceStatus::Accepted),
            Err(DomainError::InvalidStatusTransition { .. })
        ));
    }
}
