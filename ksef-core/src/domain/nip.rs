use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Polish tax identification number (Numer Identyfikacji Podatkowej).
///
/// Always stored as exactly 10 digits. Validates the checksum on construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Nip(String);

impl Nip {
    const WEIGHTS: [u32; 9] = [6, 5, 7, 2, 3, 4, 5, 6, 7];

    /// Parse and validate a NIP string.
    ///
    /// Accepts raw digits (`"1234567890"`) or formatted with dashes (`"123-456-78-90"`).
    /// Strips all non-digit characters before validation.
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let digits: String = input.chars().filter(char::is_ascii_digit).collect();

        if digits.len() != 10 {
            return Err(DomainError::InvalidNip {
                value: input.to_string(),
                reason: "must be exactly 10 digits",
            });
        }

        // Safe: we just verified all chars are ASCII digits
        let digit_values: Vec<u32> = digits.bytes().map(|b| u32::from(b - b'0')).collect();

        let checksum: u32 = digit_values
            .iter()
            .zip(Self::WEIGHTS.iter())
            .map(|(d, w)| d * w)
            .sum::<u32>()
            % 11;

        if checksum != digit_values[9] {
            return Err(DomainError::InvalidNip {
                value: input.to_string(),
                reason: "checksum validation failed",
            });
        }

        Ok(Self(digits))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Nip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Nip {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for Nip {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Nip> for String {
    fn from(nip: Nip) -> Self {
        nip.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Valid NIP tests ---

    #[test]
    fn parse_valid_nip_raw_digits() {
        // NIP of Ministerstwo Finansów (well-known valid NIP)
        let nip = Nip::parse("5260250274").unwrap();
        assert_eq!(nip.as_str(), "5260250274");
    }

    #[test]
    fn parse_valid_nip_with_dashes() {
        let nip = Nip::parse("526-025-02-74").unwrap();
        assert_eq!(nip.as_str(), "5260250274");
    }

    #[test]
    fn parse_valid_nip_with_spaces() {
        let nip = Nip::parse("526 025 02 74").unwrap();
        assert_eq!(nip.as_str(), "5260250274");
    }

    // --- Invalid NIP tests ---

    #[test]
    fn parse_nip_too_short_returns_error() {
        let err = Nip::parse("123456789").unwrap_err();
        match err {
            DomainError::InvalidNip { reason, .. } => {
                assert_eq!(reason, "must be exactly 10 digits");
            }
            other => panic!("expected InvalidNip, got {other:?}"),
        }
    }

    #[test]
    fn parse_nip_too_long_returns_error() {
        let err = Nip::parse("12345678901").unwrap_err();
        match err {
            DomainError::InvalidNip { reason, .. } => {
                assert_eq!(reason, "must be exactly 10 digits");
            }
            other => panic!("expected InvalidNip, got {other:?}"),
        }
    }

    #[test]
    fn parse_nip_with_letters_returns_error() {
        let err = Nip::parse("526025AB74").unwrap_err();
        match err {
            DomainError::InvalidNip { reason, .. } => {
                assert_eq!(reason, "must be exactly 10 digits");
            }
            other => panic!("expected InvalidNip, got {other:?}"),
        }
    }

    #[test]
    fn parse_nip_bad_checksum_returns_error() {
        // Change last digit to make checksum invalid
        let err = Nip::parse("5260250275").unwrap_err();
        match err {
            DomainError::InvalidNip { reason, .. } => {
                assert_eq!(reason, "checksum validation failed");
            }
            other => panic!("expected InvalidNip checksum error, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_string_returns_error() {
        assert!(Nip::parse("").is_err());
    }

    // --- Display / FromStr round-trip ---

    #[test]
    fn display_and_from_str_round_trip() {
        let nip = Nip::parse("5260250274").unwrap();
        let displayed = nip.to_string();
        let parsed_back: Nip = displayed.parse().unwrap();
        assert_eq!(nip, parsed_back);
    }

    // --- Serde round-trip ---

    #[test]
    fn serde_json_round_trip() {
        let nip = Nip::parse("5260250274").unwrap();
        let json = serde_json::to_string(&nip).unwrap();
        assert_eq!(json, "\"5260250274\"");
        let deserialized: Nip = serde_json::from_str(&json).unwrap();
        assert_eq!(nip, deserialized);
    }

    #[test]
    fn serde_json_invalid_nip_returns_error() {
        // NIP with bad checksum — last digit should not pass validation
        let result: Result<Nip, _> = serde_json::from_str("\"1234567890\"");
        assert!(result.is_err());
    }

    // --- Equality ---

    #[test]
    fn same_nip_different_format_are_equal() {
        let a = Nip::parse("5260250274").unwrap();
        let b = Nip::parse("526-025-02-74").unwrap();
        assert_eq!(a, b);
    }
}
