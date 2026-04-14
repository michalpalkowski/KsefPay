use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::nip::Nip;
use crate::error::DomainError;

/// Full set of `KSeF` permission codes supported by this project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PermissionType {
    InvoiceRead,
    InvoiceWrite,
    Introspection,
    CredentialsRead,
    CredentialsManage,
    EnforcementOperations,
    SubunitManage,
}

impl fmt::Display for PermissionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvoiceRead => write!(f, "InvoiceRead"),
            Self::InvoiceWrite => write!(f, "InvoiceWrite"),
            Self::Introspection => write!(f, "Introspection"),
            Self::CredentialsRead => write!(f, "CredentialsRead"),
            Self::CredentialsManage => write!(f, "CredentialsManage"),
            Self::EnforcementOperations => write!(f, "EnforcementOperations"),
            Self::SubunitManage => write!(f, "SubunitManage"),
        }
    }
}

impl FromStr for PermissionType {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "invoiceread" | "invoice_read" => Ok(Self::InvoiceRead),
            "invoicewrite" | "invoice_write" => Ok(Self::InvoiceWrite),
            "introspection" => Ok(Self::Introspection),
            "credentialsread" | "credentials_read" => Ok(Self::CredentialsRead),
            "credentialsmanage" | "credentials_manage" => Ok(Self::CredentialsManage),
            "enforcementoperations" | "enforcement_operations" => Ok(Self::EnforcementOperations),
            "subunitmanage" | "subunit_manage" => Ok(Self::SubunitManage),
            other => Err(DomainError::InvalidParse {
                type_name: "PermissionType",
                value: other.to_string(),
            }),
        }
    }
}

/// Request to grant or revoke `KSeF` permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionChangeRequest {
    pub context_nip: Nip,
    pub authorized_nip: Nip,
    pub permissions: Vec<PermissionType>,
}

impl PermissionChangeRequest {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.permissions.is_empty() {
            return Err(DomainError::InvalidParse {
                type_name: "PermissionChangeRequest.permissions",
                value: "empty".to_string(),
            });
        }
        Ok(())
    }
}

/// Type aliases for backward-compatible naming at call sites.
pub type PermissionGrantRequest = PermissionChangeRequest;
pub type PermissionRevokeRequest = PermissionChangeRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRecord {
    pub permission: PermissionType,
    pub granted_at: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_nip() -> Nip {
        Nip::parse("5260250274").unwrap()
    }

    #[test]
    fn permission_type_roundtrip_display_parse() {
        let all = [
            PermissionType::InvoiceRead,
            PermissionType::InvoiceWrite,
            PermissionType::Introspection,
            PermissionType::CredentialsRead,
            PermissionType::CredentialsManage,
            PermissionType::EnforcementOperations,
            PermissionType::SubunitManage,
        ];

        for permission in all {
            let rendered = permission.to_string();
            let parsed: PermissionType = rendered.parse().unwrap();
            assert_eq!(permission, parsed);
        }
    }

    #[test]
    fn permission_type_unknown_returns_error() {
        assert!(matches!(
            "unknown".parse::<PermissionType>(),
            Err(DomainError::InvalidParse {
                type_name: "PermissionType",
                ..
            })
        ));
    }

    #[test]
    fn change_request_validate_rejects_empty_permissions() {
        let request = PermissionChangeRequest {
            context_nip: test_nip(),
            authorized_nip: test_nip(),
            permissions: vec![],
        };

        assert!(matches!(
            request.validate(),
            Err(DomainError::InvalidParse {
                type_name: "PermissionChangeRequest.permissions",
                ..
            })
        ));
    }

    #[test]
    fn change_request_validate_accepts_non_empty_permissions() {
        let request = PermissionChangeRequest {
            context_nip: test_nip(),
            authorized_nip: test_nip(),
            permissions: vec![PermissionType::InvoiceRead],
        };

        assert!(request.validate().is_ok());
    }
}
