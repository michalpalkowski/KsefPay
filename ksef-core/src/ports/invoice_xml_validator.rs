use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

/// Port: validate outbound invoice XML against the authoritative schema.
pub trait InvoiceXmlValidator: Send + Sync {
    /// Validate serialized invoice XML before submission to KSeF.
    fn validate(&self, xml: &InvoiceXml) -> Result<(), XmlError>;
}
