use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::domain::nip::Nip;
use crate::domain::user::UserId;

/// Audited security-sensitive actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    Login,
    Register,
    CreateInvoice,
    SubmitInvoice,
    FetchInvoices,
    GrantPermission,
    RevokePermission,
    GenerateToken,
    RevokeToken,
    SaveCertificate,
    DeleteCertificate,
    ExportStart,
    ChangePassword,
}

impl AuditAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Register => "register",
            Self::CreateInvoice => "create_invoice",
            Self::SubmitInvoice => "submit_invoice",
            Self::FetchInvoices => "fetch_invoices",
            Self::GrantPermission => "grant_permission",
            Self::RevokePermission => "revoke_permission",
            Self::GenerateToken => "generate_token",
            Self::RevokeToken => "revoke_token",
            Self::SaveCertificate => "save_certificate",
            Self::DeleteCertificate => "delete_certificate",
            Self::ExportStart => "export_start",
            Self::ChangePassword => "change_password",
        }
    }
}

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AuditAction {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "login" => Ok(Self::Login),
            "register" => Ok(Self::Register),
            "create_invoice" => Ok(Self::CreateInvoice),
            "submit_invoice" => Ok(Self::SubmitInvoice),
            "fetch_invoices" => Ok(Self::FetchInvoices),
            "grant_permission" => Ok(Self::GrantPermission),
            "revoke_permission" => Ok(Self::RevokePermission),
            "generate_token" => Ok(Self::GenerateToken),
            "revoke_token" => Ok(Self::RevokeToken),
            "save_certificate" => Ok(Self::SaveCertificate),
            "delete_certificate" => Ok(Self::DeleteCertificate),
            "export_start" => Ok(Self::ExportStart),
            "change_password" => Ok(Self::ChangePassword),
            other => Err(format!("invalid audit action: '{other}'")),
        }
    }
}

/// Persisted audit log entry.
#[derive(Debug, Clone)]
pub struct AuditLogEntry {
    pub id: uuid::Uuid,
    pub timestamp: DateTime<Utc>,
    pub user_id: UserId,
    pub user_email: String,
    pub nip: Option<Nip>,
    pub action: AuditAction,
    pub details: Option<String>,
    pub ip_address: Option<String>,
}

/// New audit entry to be persisted.
#[derive(Debug, Clone)]
pub struct NewAuditLogEntry {
    pub user_id: UserId,
    pub user_email: String,
    pub nip: Option<Nip>,
    pub action: AuditAction,
    pub details: Option<String>,
    pub ip_address: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_action_roundtrip() {
        let action = AuditAction::GenerateToken;
        let parsed: AuditAction = action.to_string().parse().unwrap();
        assert_eq!(parsed, action);
    }

    #[test]
    fn audit_action_rejects_unknown_value() {
        assert!("definitely_not_real".parse::<AuditAction>().is_err());
    }
}
