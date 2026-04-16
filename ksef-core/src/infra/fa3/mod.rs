mod generated;
mod model_converter;
mod parse;
mod serialize;
mod versioned_adapter;
mod xsd_validator;

use crate::domain::invoice::{Direction, Invoice};
use crate::domain::session::KSeFNumber;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;
use crate::ports::invoice_xml::InvoiceXmlConverter;

pub use parse::xml_to_invoice;
pub use serialize::invoice_to_xml;
pub use versioned_adapter::{current_adapter as current_fa3_adapter, Fa3VersionAdapter};
pub use xsd_validator::Fa3XsdValidator;

/// FA(3) adapter implementing the XML conversion port.
pub struct Fa3XmlConverter;

impl InvoiceXmlConverter for Fa3XmlConverter {
    fn to_xml(&self, invoice: &Invoice) -> Result<InvoiceXml, XmlError> {
        invoice_to_xml(invoice)
    }

    fn from_xml(
        &self,
        xml: &InvoiceXml,
        direction: Direction,
        ksef_number: &KSeFNumber,
    ) -> Result<Invoice, XmlError> {
        xml_to_invoice(xml, direction, ksef_number)
    }
}
