use crate::domain::invoice::Invoice;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

use super::versioned_adapter::current_adapter;

/// Serialize a domain `Invoice` into FA(3) compliant XML.
pub fn invoice_to_xml(invoice: &Invoice) -> Result<InvoiceXml, XmlError> {
    current_adapter().to_xml(invoice)
}
