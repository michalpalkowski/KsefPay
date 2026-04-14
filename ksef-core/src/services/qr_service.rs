use std::sync::Arc;

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::domain::environment::KSeFEnvironment;
use crate::domain::invoice::Invoice;
use crate::domain::qr::{KodI, KodII, QRCodeData, QRCodeOptions};
use crate::ports::qr_renderer::QrRenderer;

#[derive(Debug, thiserror::Error)]
pub enum QrServiceError {
    #[error("invoice seller NIP is required for KOD I generation")]
    MissingSellerNip,

    #[error("invoice raw XML is required for KOD I generation")]
    MissingRawXml,

    #[error("certificate serial is required for KOD II generation")]
    MissingCertificateSerial,

    #[error("QR generation failed: {0}")]
    Generation(String),
}

pub struct QRService {
    environment: KSeFEnvironment,
    renderer: Arc<dyn QrRenderer>,
}

impl QRService {
    #[must_use]
    pub fn new(environment: KSeFEnvironment, renderer: Arc<dyn QrRenderer>) -> Self {
        Self {
            environment,
            renderer,
        }
    }

    #[must_use]
    pub fn environment(&self) -> KSeFEnvironment {
        self.environment
    }

    pub fn build_kod_i(&self, invoice: &Invoice) -> Result<KodI, QrServiceError> {
        let nip = invoice
            .seller
            .nip
            .as_ref()
            .ok_or(QrServiceError::MissingSellerNip)?
            .as_str();
        let xml = invoice
            .raw_xml
            .as_ref()
            .ok_or(QrServiceError::MissingRawXml)?;
        let hash = sha256_base64url(xml.as_bytes());
        let date = invoice.issue_date.format("%d-%m-%Y");
        let url = format!(
            "https://{}/invoice/{nip}/{date}/{hash}",
            qr_host(self.environment)
        );
        let qr_data = QRCodeData { url };
        qr_data
            .validate()
            .map_err(|e| QrServiceError::Generation(format!("{e}")))?;
        Ok(KodI(qr_data))
    }

    pub fn build_kod_ii(
        &self,
        invoice: &Invoice,
        certificate_serial: &str,
    ) -> Result<KodII, QrServiceError> {
        if certificate_serial.trim().is_empty() {
            return Err(QrServiceError::MissingCertificateSerial);
        }
        let kod_i = self.build_kod_i(invoice)?;
        let signature =
            sha256_base64url(format!("{certificate_serial}:{}", (kod_i.0).url).as_bytes());
        let url = format!(
            "https://{}/offline?certSerial={}&signature={}",
            qr_host(self.environment),
            certificate_serial,
            signature
        );
        let qr_data = QRCodeData { url };
        qr_data
            .validate()
            .map_err(|e| QrServiceError::Generation(format!("{e}")))?;
        Ok(KodII(qr_data))
    }

    pub fn render_kod_i_png(
        &self,
        invoice: &Invoice,
        options: QRCodeOptions,
    ) -> Result<Vec<u8>, QrServiceError> {
        let kod = self.build_kod_i(invoice)?;
        self.renderer
            .render_png(&kod.0, options)
            .map_err(|e| QrServiceError::Generation(e.to_string()))
    }

    pub fn render_kod_i_svg(
        &self,
        invoice: &Invoice,
        options: QRCodeOptions,
    ) -> Result<String, QrServiceError> {
        let kod = self.build_kod_i(invoice)?;
        self.renderer
            .render_svg(&kod.0, options)
            .map_err(|e| QrServiceError::Generation(e.to_string()))
    }
}

fn qr_host(environment: KSeFEnvironment) -> &'static str {
    match environment {
        KSeFEnvironment::Test => "qr-test.ksef.mf.gov.pl",
        KSeFEnvironment::Demo => "qr-demo.ksef.mf.gov.pl",
        KSeFEnvironment::Production => "qr.ksef.mf.gov.pl",
    }
}

fn sha256_base64url(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::invoice::{
        Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
        LineItem, Money, Party, Quantity, VatRate,
    };
    use crate::domain::nip::Nip;
    use crate::infra::qr::generator::QRCodeGenerator;

    fn service() -> QRService {
        QRService::new(KSeFEnvironment::Test, Arc::new(QRCodeGenerator))
    }

    fn invoice_with_xml() -> Invoice {
        let nip = Nip::parse("5260250274").unwrap();
        Invoice {
            id: InvoiceId::new(),
            direction: Direction::Outgoing,
            status: InvoiceStatus::Draft,
            invoice_type: InvoiceType::Vat,
            invoice_number: "INV-1".to_string(),
            issue_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 13).unwrap(),
            sale_date: None,
            corrected_invoice_number: None,
            correction_reason: None,
            original_ksef_number: None,
            advance_payment_date: None,
            seller: Party {
                nip: Some(nip.clone()),
                name: "Seller".to_string(),
                address: Address {
                    country_code: CountryCode::pl(),
                    line1: "l1".to_string(),
                    line2: String::new(),
                },
            },
            buyer: Party {
                nip: Some(nip),
                name: "Buyer".to_string(),
                address: Address {
                    country_code: CountryCode::pl(),
                    line1: "l2".to_string(),
                    line2: String::new(),
                },
            },
            currency: Currency::pln(),
            line_items: vec![LineItem {
                line_number: 1,
                description: "item".to_string(),
                unit: Some("szt".to_string()),
                quantity: Quantity::integer(1),
                unit_net_price: Some(Money::from_pln(1, 0)),
                net_value: Money::from_pln(1, 0),
                vat_rate: VatRate::Rate23,
                vat_amount: Money::from_pln(0, 23),
                gross_value: Money::from_pln(1, 23),
            }],
            total_net: Money::from_pln(1, 0),
            total_vat: Money::from_pln(0, 23),
            total_gross: Money::from_pln(1, 23),
            payment_method: None,
            payment_deadline: None,
            bank_account: None,
            ksef_number: None,
            ksef_error: None,
            raw_xml: Some("<Faktura>test</Faktura>".to_string()),
        }
    }

    #[test]
    fn kod_i_url_matches_required_pattern() {
        let service = service();
        let invoice = invoice_with_xml();
        let kod_i = service.build_kod_i(&invoice).unwrap();
        let url = &(kod_i.0).url;
        assert!(url.starts_with("https://qr-test.ksef.mf.gov.pl/invoice/5260250274/13-04-2026/"));
    }

    #[test]
    fn missing_raw_xml_fails_fast() {
        let service = service();
        let mut invoice = invoice_with_xml();
        invoice.raw_xml = None;
        let err = service.build_kod_i(&invoice).unwrap_err();
        assert!(matches!(err, QrServiceError::MissingRawXml));
    }

    #[test]
    fn render_png_and_svg_are_valid() {
        let service = service();
        let invoice = invoice_with_xml();

        let png = service
            .render_kod_i_png(&invoice, QRCodeOptions::default())
            .unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));

        let svg = service
            .render_kod_i_svg(&invoice, QRCodeOptions::default())
            .unwrap();
        assert!(svg.contains("<svg"));
    }
}
