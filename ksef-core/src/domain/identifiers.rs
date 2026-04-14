use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pesel(String);

impl Pesel {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let trimmed = input.trim();
        if trimmed.len() != 11 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Err(DomainError::InvalidParse {
                type_name: "Pesel",
                value: input.to_string(),
            });
        }

        let digits: Vec<u32> = trimmed
            .chars()
            .map(|c| c.to_digit(10).expect("already validated as digit"))
            .collect();
        let weights = [1_u32, 3, 7, 9, 1, 3, 7, 9, 1, 3];
        let weighted_sum: u32 = digits
            .iter()
            .take(10)
            .zip(weights)
            .map(|(digit, weight)| digit * weight)
            .sum();
        let checksum = (10 - (weighted_sum % 10)) % 10;
        if checksum != digits[10] {
            return Err(DomainError::InvalidParse {
                type_name: "Pesel",
                value: input.to_string(),
            });
        }

        Ok(Self(trimmed.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Pesel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Pesel {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NipVatUe(String);

const EU_COUNTRY_CODES: [&str; 27] = [
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "EL", "ES", "FI", "FR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
];

impl NipVatUe {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let normalized = input.trim().to_ascii_uppercase();
        if normalized.len() < 4 {
            return Err(DomainError::InvalidParse {
                type_name: "NipVatUe",
                value: input.to_string(),
            });
        }

        let (country, rest) = normalized.split_at(2);
        let country_valid = EU_COUNTRY_CODES.contains(&country);
        let rest_valid = rest.chars().all(|c| c.is_ascii_alphanumeric());
        if !country_valid || !rest_valid {
            return Err(DomainError::InvalidParse {
                type_name: "NipVatUe",
                value: input.to_string(),
            });
        }

        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NipVatUe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for NipVatUe {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeppolId(String);

impl PeppolId {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let trimmed = input.trim();
        let has_separator = trimmed.contains(':');
        let has_whitespace = trimmed.chars().any(char::is_whitespace);
        if trimmed.is_empty() || !has_separator || has_whitespace {
            return Err(DomainError::InvalidParse {
                type_name: "PeppolId",
                value: input.to_string(),
            });
        }

        Ok(Self(trimmed.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PeppolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PeppolId {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InternalId(String);

impl InternalId {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err(DomainError::InvalidParse {
                type_name: "InternalId",
                value: input.to_string(),
            });
        }

        Ok(Self(trimmed.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InternalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for InternalId {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint(String);

impl Fingerprint {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let normalized = input.trim().replace(':', "").to_ascii_uppercase();
        let allowed_len = normalized.len() == 40 || normalized.len() == 64;
        let all_hex = normalized.chars().all(|c| c.is_ascii_hexdigit());
        if !allowed_len || !all_hex {
            return Err(DomainError::InvalidParse {
                type_name: "Fingerprint",
                value: input.to_string(),
            });
        }

        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Fingerprint {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pesel_valid_checksum() {
        let pesel = Pesel::parse("44051401359").unwrap();
        assert_eq!(pesel.as_str(), "44051401359");
    }

    #[test]
    fn pesel_invalid_returns_error() {
        assert!(matches!(
            Pesel::parse("44051401358"),
            Err(DomainError::InvalidParse {
                type_name: "Pesel",
                ..
            })
        ));
    }

    #[test]
    fn nip_vat_ue_parses_and_normalizes_case() {
        let vat = NipVatUe::parse("pl1234567890").unwrap();
        assert_eq!(vat.as_str(), "PL1234567890");
    }

    #[test]
    fn nip_vat_ue_rejects_non_eu_prefix() {
        assert!(matches!(
            NipVatUe::parse("US123456"),
            Err(DomainError::InvalidParse {
                type_name: "NipVatUe",
                ..
            })
        ));
    }

    #[test]
    fn peppol_id_requires_separator() {
        assert!(matches!(
            PeppolId::parse("0088-123456789"),
            Err(DomainError::InvalidParse {
                type_name: "PeppolId",
                ..
            })
        ));
    }

    #[test]
    fn peppol_id_valid_example() {
        let id = PeppolId::parse("iso6523-actorid-upis::0088:1234567890123").unwrap();
        assert_eq!(id.as_str(), "iso6523-actorid-upis::0088:1234567890123");
    }

    #[test]
    fn internal_id_rejects_empty() {
        assert!(matches!(
            InternalId::parse(""),
            Err(DomainError::InvalidParse {
                type_name: "InternalId",
                ..
            })
        ));
    }

    #[test]
    fn internal_id_accepts_trimmed_value() {
        let id = InternalId::parse("  INV-001  ").unwrap();
        assert_eq!(id.as_str(), "INV-001");
    }

    #[test]
    fn fingerprint_accepts_colon_format() {
        let fp = Fingerprint::parse("AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD")
            .unwrap();
        assert_eq!(fp.as_str(), "AABBCCDDEEFF00112233445566778899AABBCCDD");
    }

    #[test]
    fn fingerprint_rejects_invalid_chars() {
        assert!(matches!(
            Fingerprint::parse("ZZ11"),
            Err(DomainError::InvalidParse {
                type_name: "Fingerprint",
                ..
            })
        ));
    }
}
