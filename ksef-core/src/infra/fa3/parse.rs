use crate::domain::invoice::{Direction, Invoice};
use crate::domain::session::KSeFNumber;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

use super::versioned_adapter::current_adapter;

/// Parse a FA(3) XML document into a domain `Invoice`.
pub fn xml_to_invoice(
    xml: &InvoiceXml,
    direction: Direction,
    ksef_number: &KSeFNumber,
) -> Result<Invoice, XmlError> {
    current_adapter().from_xml(xml, direction, ksef_number)
}
