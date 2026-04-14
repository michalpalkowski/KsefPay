use crate::domain::invoice::{Direction, Invoice};
use crate::domain::session::KSeFNumber;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

/// Port: convert between domain `Invoice` and FA(3) XML.
pub trait InvoiceXmlConverter: Send + Sync {
    /// Serialize a domain invoice into FA(3) XML.
    fn to_xml(&self, invoice: &Invoice) -> Result<InvoiceXml, XmlError>;

    /// Parse FA(3) XML into a domain invoice.
    fn from_xml(
        &self,
        xml: &InvoiceXml,
        direction: Direction,
        ksef_number: &KSeFNumber,
    ) -> Result<Invoice, XmlError>;
}
