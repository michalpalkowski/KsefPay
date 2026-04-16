use crate::domain::invoice::{Direction, Invoice};
use crate::domain::session::KSeFNumber;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

use super::model_converter;

pub trait Fa3VersionAdapter: Send + Sync {
    fn schema_id(&self) -> &'static str;
    fn to_xml(&self, invoice: &Invoice) -> Result<InvoiceXml, XmlError>;
    fn from_xml(
        &self,
        xml: &InvoiceXml,
        direction: Direction,
        ksef_number: &KSeFNumber,
    ) -> Result<Invoice, XmlError>;
}

pub struct Fa3V2025_06_25_13775Adapter;

impl Fa3VersionAdapter for Fa3V2025_06_25_13775Adapter {
    fn schema_id(&self) -> &'static str {
        "2025-06-25-13775"
    }

    fn to_xml(&self, invoice: &Invoice) -> Result<InvoiceXml, XmlError> {
        model_converter::invoice_to_xml(invoice)
    }

    fn from_xml(
        &self,
        xml: &InvoiceXml,
        direction: Direction,
        ksef_number: &KSeFNumber,
    ) -> Result<Invoice, XmlError> {
        model_converter::xml_to_invoice(xml, direction, ksef_number)
    }
}

pub static CURRENT_ADAPTER: Fa3V2025_06_25_13775Adapter = Fa3V2025_06_25_13775Adapter;

#[must_use]
pub fn current_adapter() -> &'static dyn Fa3VersionAdapter {
    &CURRENT_ADAPTER
}
