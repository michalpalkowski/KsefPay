use std::fmt;

use serde::{Deserialize, Serialize};

use super::nip::Nip;

/// Type of `KSeF` certificate, determines which OID field carries the NIP.
///
/// `KSeF` recognizes two distinct certificate types, each with NIP stored
/// in a different X.509 Distinguished Name field:
///
/// | Type | OID | Field name | Prefix | Use case |
/// |------|-----|------------|--------|----------|
/// | Seal | 2.5.4.97 | `organizationIdentifier` | `VATPL-` | Company systems, automated |
/// | Personal | 2.5.4.5 | `serialNumber` | `TINPL-` | Person acting on behalf of company |
///
/// Source: CIRFMF/ksef-docs — `certyfikaty-KSeF.md`, `uwierzytelnianie.md`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateKind {
    /// Company seal (pieczec firmowa).
    /// NIP in `organizationIdentifier` (OID 2.5.4.97), prefix `VATPL-`.
    /// Generated via `CertificateUtils.GetCompanySeal()` in official SDK.
    Seal,

    /// Personal certificate (certyfikat osoby fizycznej).
    /// NIP in `serialNumber` (OID 2.5.4.5), prefix `TINPL-`.
    /// Generated via `CertificateUtils.GetPersonalCertificate()` in official SDK.
    Personal,
}

impl CertificateKind {
    /// X.509 OID where NIP is stored.
    #[must_use]
    pub fn nip_oid(self) -> &'static str {
        match self {
            Self::Seal => "2.5.4.97",
            Self::Personal => "2.5.4.5",
        }
    }

    /// X.509 field name.
    #[must_use]
    pub fn field_name(self) -> &'static str {
        match self {
            Self::Seal => "organizationIdentifier",
            Self::Personal => "serialNumber",
        }
    }

    /// Prefix before NIP value in the certificate field.
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Seal => "VATPL-",
            Self::Personal => "TINPL-",
        }
    }

    /// Format NIP with the correct prefix for this certificate kind.
    #[must_use]
    pub fn format_nip(self, nip: &Nip) -> String {
        format!("{}{}", self.prefix(), nip.as_str())
    }
}

impl fmt::Display for CertificateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seal => write!(f, "seal"),
            Self::Personal => write!(f, "personal"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_certificate_uses_vatpl_prefix() {
        let nip = Nip::parse("5260250274").unwrap();
        assert_eq!(CertificateKind::Seal.format_nip(&nip), "VATPL-5260250274");
        assert_eq!(CertificateKind::Seal.field_name(), "organizationIdentifier");
        assert_eq!(CertificateKind::Seal.nip_oid(), "2.5.4.97");
    }

    #[test]
    fn personal_certificate_uses_tinpl_prefix() {
        let nip = Nip::parse("5260250274").unwrap();
        assert_eq!(
            CertificateKind::Personal.format_nip(&nip),
            "TINPL-5260250274"
        );
        assert_eq!(CertificateKind::Personal.field_name(), "serialNumber");
        assert_eq!(CertificateKind::Personal.nip_oid(), "2.5.4.5");
    }

    #[test]
    fn cannot_confuse_prefixes() {
        // Protocol enforces the correct prefix — no way to pass TINPL to a Seal
        assert_ne!(
            CertificateKind::Seal.prefix(),
            CertificateKind::Personal.prefix()
        );
    }
}
