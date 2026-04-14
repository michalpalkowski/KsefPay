use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeppolProvider {
    pub provider_id: String,
    pub name: String,
    pub country_code: String,
    pub endpoint_url: String,
    pub active: bool,
}

impl PeppolProvider {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !self.endpoint_url.starts_with("https://") {
            return Err(DomainError::InvalidParse {
                type_name: "PeppolProvider.endpoint_url",
                value: self.endpoint_url.clone(),
            });
        }
        if self.country_code.len() != 2
            || !self.country_code.chars().all(|c| c.is_ascii_uppercase())
        {
            return Err(DomainError::InvalidParse {
                type_name: "PeppolProvider.country_code",
                value: self.country_code.clone(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peppol_provider_validate_accepts_https_and_iso_country() {
        let provider = PeppolProvider {
            provider_id: "provider-1".to_string(),
            name: "Provider".to_string(),
            country_code: "PL".to_string(),
            endpoint_url: "https://example.org/peppol".to_string(),
            active: true,
        };
        assert!(provider.validate().is_ok());
    }

    #[test]
    fn peppol_provider_validate_rejects_invalid_endpoint() {
        let provider = PeppolProvider {
            provider_id: "provider-1".to_string(),
            name: "Provider".to_string(),
            country_code: "PL".to_string(),
            endpoint_url: "http://example.org/peppol".to_string(),
            active: true,
        };
        assert!(matches!(
            provider.validate(),
            Err(DomainError::InvalidParse {
                type_name: "PeppolProvider.endpoint_url",
                ..
            })
        ));
    }
}
