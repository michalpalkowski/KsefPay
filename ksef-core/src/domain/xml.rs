use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Invoice XML received from an external source.
///
/// This type is intentionally separate from `InvoiceXml` so trust boundaries are
/// explicit in function signatures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UntrustedInvoiceXml(String);

impl UntrustedInvoiceXml {
    #[must_use]
    pub fn new(content: String) -> Self {
        Self(content)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

/// Invoice XML content (plaintext FA(3) XML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceXml(String);

impl InvoiceXml {
    const MAX_UNTRUSTED_XML_BYTES: usize = 2 * 1024 * 1024;

    #[must_use]
    pub(crate) fn new(content: String) -> Self {
        Self(content)
    }

    /// Construct XML from untrusted external input (e.g. downloaded payloads).
    ///
    /// This applies strict checks so the payload can only be treated as passive data.
    pub fn from_untrusted(untrusted: UntrustedInvoiceXml) -> Result<Self, DomainError> {
        Self::try_from(untrusted)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<UntrustedInvoiceXml> for InvoiceXml {
    type Error = DomainError;

    fn try_from(value: UntrustedInvoiceXml) -> Result<Self, Self::Error> {
        validate_untrusted_xml(value.as_str())?;
        Ok(Self(value.into_string()))
    }
}

impl From<InvoiceXml> for UntrustedInvoiceXml {
    fn from(value: InvoiceXml) -> Self {
        Self(value.into_string())
    }
}

fn validate_untrusted_xml(content: &str) -> Result<(), DomainError> {
    if content.is_empty() {
        return Err(DomainError::InvalidParse {
            type_name: "InvoiceXml",
            value: "empty XML payload".to_string(),
        });
    }

    if content.len() > InvoiceXml::MAX_UNTRUSTED_XML_BYTES {
        return Err(DomainError::InvalidParse {
            type_name: "InvoiceXml",
            value: format!(
                "payload too large: {} bytes (limit: {} bytes)",
                content.len(),
                InvoiceXml::MAX_UNTRUSTED_XML_BYTES
            ),
        });
    }

    if content.contains('\0') {
        return Err(DomainError::InvalidParse {
            type_name: "InvoiceXml",
            value: "payload contains NUL byte".to_string(),
        });
    }

    let lowered = content.to_ascii_lowercase();
    for marker in ["<!doctype", "<!entity", "<?xml-stylesheet"] {
        if lowered.contains(marker) {
            return Err(DomainError::InvalidParse {
                type_name: "InvoiceXml",
                value: format!("forbidden XML construct detected: {marker}"),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoice_xml_encapsulated() {
        let xml = InvoiceXml::new("<Faktura/>".to_string());
        assert_eq!(xml.as_str(), "<Faktura/>");
        assert_eq!(xml.as_bytes(), b"<Faktura/>");
    }

    #[test]
    fn from_untrusted_accepts_regular_fa_xml() {
        let xml = InvoiceXml::from_untrusted(UntrustedInvoiceXml::new(
            "<Faktura><Fa/></Faktura>".to_string(),
        ))
        .unwrap();
        assert_eq!(xml.as_str(), "<Faktura><Fa/></Faktura>");
    }

    #[test]
    fn from_untrusted_rejects_doctype_and_entities() {
        let err = InvoiceXml::from_untrusted(UntrustedInvoiceXml::new(
            r#"<?xml version="1.0"?><!DOCTYPE foo [ <!ENTITY xxe SYSTEM "file:///etc/passwd"> ]><Faktura>&xxe;</Faktura>"#
                .to_string(),
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidParse {
                type_name: "InvoiceXml",
                ..
            }
        ));
    }

    #[test]
    fn from_untrusted_rejects_xml_stylesheet_pi() {
        let err = InvoiceXml::from_untrusted(UntrustedInvoiceXml::new(
            r#"<?xml version="1.0"?><?xml-stylesheet type="text/xsl" href="evil.xsl"?><Faktura/>"#
                .to_string(),
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidParse {
                type_name: "InvoiceXml",
                ..
            }
        ));
    }
}
