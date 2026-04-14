use chrono::NaiveDate;

use crate::error::DomainError;

pub fn validate_email(email: &str) -> Result<(), DomainError> {
    let trimmed = email.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return Err(DomainError::InvalidParse {
            type_name: "Email",
            value: email.to_string(),
        });
    }

    let mut parts = trimmed.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if local.is_empty()
        || domain.is_empty()
        || parts.next().is_some()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return Err(DomainError::InvalidParse {
            type_name: "Email",
            value: email.to_string(),
        });
    }

    Ok(())
}

pub fn validate_phone(phone: &str) -> Result<(), DomainError> {
    let trimmed = phone.trim();
    if trimmed.is_empty() {
        return Err(DomainError::InvalidParse {
            type_name: "Phone",
            value: phone.to_string(),
        });
    }

    let digits = if let Some(rest) = trimmed.strip_prefix('+') {
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
            return Err(DomainError::InvalidParse {
                type_name: "Phone",
                value: phone.to_string(),
            });
        }
        rest
    } else if trimmed.chars().all(|c| c.is_ascii_digit()) {
        trimmed
    } else {
        return Err(DomainError::InvalidParse {
            type_name: "Phone",
            value: phone.to_string(),
        });
    };

    if !(7..=15).contains(&digits.len()) {
        return Err(DomainError::InvalidParse {
            type_name: "Phone",
            value: phone.to_string(),
        });
    }

    Ok(())
}

pub fn validate_iso_country_code(country_code: &str) -> Result<(), DomainError> {
    let trimmed = country_code.trim();
    if trimmed.len() != 2 || !trimmed.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(DomainError::InvalidParse {
            type_name: "IsoCountryCode",
            value: country_code.to_string(),
        });
    }
    Ok(())
}

pub fn validate_file_size(file_size_bytes: u64, max_size_bytes: u64) -> Result<(), DomainError> {
    if file_size_bytes > max_size_bytes {
        return Err(DomainError::InvalidParse {
            type_name: "FileSize",
            value: format!("{file_size_bytes}>{max_size_bytes}"),
        });
    }
    Ok(())
}

pub fn validate_date_range(from: NaiveDate, to: NaiveDate) -> Result<(), DomainError> {
    if from > to {
        return Err(DomainError::InvalidParse {
            type_name: "DateRange",
            value: format!("{from}>{to}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_email_accepts_valid_address() {
        assert!(validate_email("john.doe@example.com").is_ok());
    }

    #[test]
    fn validate_email_rejects_invalid_address() {
        assert!(matches!(
            validate_email("john.doe.example.com"),
            Err(DomainError::InvalidParse {
                type_name: "Email",
                ..
            })
        ));
    }

    #[test]
    fn validate_phone_accepts_e164_and_digits() {
        assert!(validate_phone("+48500100200").is_ok());
        assert!(validate_phone("500100200").is_ok());
    }

    #[test]
    fn validate_phone_rejects_invalid_phone() {
        assert!(matches!(
            validate_phone("+48 500100200"),
            Err(DomainError::InvalidParse {
                type_name: "Phone",
                ..
            })
        ));
        assert!(matches!(
            validate_phone("1234"),
            Err(DomainError::InvalidParse {
                type_name: "Phone",
                ..
            })
        ));
    }

    #[test]
    fn validate_iso_country_code_accepts_two_uppercase_letters() {
        assert!(validate_iso_country_code("PL").is_ok());
    }

    #[test]
    fn validate_iso_country_code_rejects_invalid_code() {
        assert!(matches!(
            validate_iso_country_code("pl"),
            Err(DomainError::InvalidParse {
                type_name: "IsoCountryCode",
                ..
            })
        ));
        assert!(matches!(
            validate_iso_country_code("POL"),
            Err(DomainError::InvalidParse {
                type_name: "IsoCountryCode",
                ..
            })
        ));
    }

    #[test]
    fn validate_file_size_accepts_file_within_limit() {
        assert!(validate_file_size(1024, 4096).is_ok());
    }

    #[test]
    fn validate_file_size_rejects_file_over_limit() {
        assert!(matches!(
            validate_file_size(4097, 4096),
            Err(DomainError::InvalidParse {
                type_name: "FileSize",
                ..
            })
        ));
    }

    #[test]
    fn validate_date_range_accepts_chronological_range() {
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert!(validate_date_range(from, to).is_ok());
    }

    #[test]
    fn validate_date_range_rejects_inverted_range() {
        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert!(matches!(
            validate_date_range(from, to),
            Err(DomainError::InvalidParse {
                type_name: "DateRange",
                ..
            })
        ));
    }
}
