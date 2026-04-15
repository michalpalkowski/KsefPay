use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::nip::Nip;

/// VAT registration status from Biała Lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VatStatus {
    /// Czynny podatnik VAT.
    Active,
    /// Zwolniony z VAT.
    Exempt,
    /// Niezarejestrowany — nie figuruje w rejestrze VAT.
    Unregistered,
}

impl VatStatus {
    /// Parse from Biała Lista `statusVat` field.
    pub fn from_whitelist(raw: &str) -> Self {
        match raw.trim() {
            "Czynny" => Self::Active,
            "Zwolniony" => Self::Exempt,
            _ => Self::Unregistered,
        }
    }
}

impl fmt::Display for VatStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => f.write_str("Czynny"),
            Self::Exempt => f.write_str("Zwolniony"),
            Self::Unregistered => f.write_str("Niezarejestrowany"),
        }
    }
}

/// Company data fetched from Biała Lista VAT (wl-api.mf.gov.pl).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyInfo {
    pub nip: Nip,
    pub name: String,
    pub address: String,
    pub bank_accounts: Vec<String>,
    pub vat_status: VatStatus,
    pub fetched_at: DateTime<Utc>,
}

impl CompanyInfo {
    /// Whether this cached entry is still fresh (within `ttl`).
    #[must_use]
    pub fn is_fresh(&self, ttl: chrono::Duration) -> bool {
        Utc::now() - self.fetched_at < ttl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vat_status_parses_all_known_values() {
        assert_eq!(VatStatus::from_whitelist("Czynny"), VatStatus::Active);
        assert_eq!(VatStatus::from_whitelist("Zwolniony"), VatStatus::Exempt);
        assert_eq!(
            VatStatus::from_whitelist("Niezarejestrowany"),
            VatStatus::Unregistered
        );
        assert_eq!(
            VatStatus::from_whitelist("something else"),
            VatStatus::Unregistered
        );
    }

    #[test]
    fn vat_status_display_roundtrips() {
        assert_eq!(VatStatus::Active.to_string(), "Czynny");
        assert_eq!(VatStatus::Exempt.to_string(), "Zwolniony");
        assert_eq!(VatStatus::Unregistered.to_string(), "Niezarejestrowany");
    }

    #[test]
    fn company_info_is_fresh_within_ttl() {
        let info = CompanyInfo {
            nip: Nip::parse("5260250274").unwrap(),
            name: "Test".to_string(),
            address: "ul. Testowa 1".to_string(),
            bank_accounts: vec![],
            vat_status: VatStatus::Active,
            fetched_at: Utc::now(),
        };
        assert!(info.is_fresh(chrono::Duration::hours(24)));
    }

    #[test]
    fn company_info_is_stale_after_ttl() {
        let info = CompanyInfo {
            nip: Nip::parse("5260250274").unwrap(),
            name: "Test".to_string(),
            address: "ul. Testowa 1".to_string(),
            bank_accounts: vec![],
            vat_status: VatStatus::Active,
            fetched_at: Utc::now() - chrono::Duration::hours(25),
        };
        assert!(!info.is_fresh(chrono::Duration::hours(24)));
    }
}
