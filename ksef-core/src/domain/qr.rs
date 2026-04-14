use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QRCodeFormat {
    Png,
    Svg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QRCodeOptions {
    pub size: u16,
    pub margin: u16,
}

impl Default for QRCodeOptions {
    fn default() -> Self {
        Self {
            size: 512,
            margin: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QRCodeData {
    pub url: String,
}

impl QRCodeData {
    pub fn validate(&self) -> Result<(), DomainError> {
        let trimmed = self.url.trim();
        if !trimmed.starts_with("https://") || !trimmed.contains("ksef.mf.gov.pl") {
            return Err(DomainError::InvalidParse {
                type_name: "QRCodeData.url",
                value: self.url.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KodI(pub QRCodeData);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KodII(pub QRCodeData);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qrcode_data_validate_accepts_ksef_https_url() {
        let data = QRCodeData {
            url: "https://qr-test.ksef.mf.gov.pl/invoice/5260250274/13-04-2026/hash".to_string(),
        };
        assert!(data.validate().is_ok());
    }

    #[test]
    fn qrcode_data_validate_rejects_non_https_or_non_ksef_url() {
        let data = QRCodeData {
            url: "http://example.com/qr".to_string(),
        };
        assert!(matches!(
            data.validate(),
            Err(DomainError::InvalidParse {
                type_name: "QRCodeData.url",
                ..
            })
        ));
    }
}
