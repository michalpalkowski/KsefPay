use chrono::{DateTime, Utc};

use crate::domain::invoice::Invoice;
use crate::domain::offline::{OfflineInvoice, OfflineInvoiceStatus, OfflineMode};
use crate::domain::qr::{KodI, KodII};
use crate::services::qr_service::{QRService, QrServiceError};

#[derive(Debug, Clone)]
pub struct OfflineConfig {
    pub offline_hours: i64,
    pub awaryjny_hours: i64,
}

impl Default for OfflineConfig {
    fn default() -> Self {
        Self {
            offline_hours: 48,
            awaryjny_hours: 72,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OfflinePackage {
    pub invoice: OfflineInvoice,
    pub kod_i: KodI,
    pub kod_ii: KodII,
}

#[derive(Debug, thiserror::Error)]
pub enum OfflineServiceError {
    #[error(transparent)]
    Qr(#[from] QrServiceError),
}

pub struct OfflineService {
    qr_service: QRService,
    config: OfflineConfig,
}

impl OfflineService {
    #[must_use]
    pub fn new(qr_service: QRService, config: OfflineConfig) -> Self {
        Self { qr_service, config }
    }

    pub fn generate_offline_package(
        &self,
        invoice: &Invoice,
        mode: OfflineMode,
        certificate_serial: &str,
        now: DateTime<Utc>,
    ) -> Result<OfflinePackage, OfflineServiceError> {
        let deadline = mode.deadline(now, self.config.offline_hours, self.config.awaryjny_hours);
        let kod_i = self.qr_service.build_kod_i(invoice)?;
        let kod_ii = self.qr_service.build_kod_ii(invoice, certificate_serial)?;

        Ok(OfflinePackage {
            invoice: OfflineInvoice {
                invoice_id: invoice.id.clone(),
                mode,
                status: OfflineInvoiceStatus::Generated,
                created_at: now,
                deadline_at: deadline,
            },
            kod_i,
            kod_ii,
        })
    }

    pub fn mark_expired_if_needed(&self, invoice: &mut OfflineInvoice, now: DateTime<Utc>) -> bool {
        if invoice.is_expired(now)
            && let Ok(new_status) = invoice.status.transition_to(OfflineInvoiceStatus::Expired)
        {
            invoice.status = new_status;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::environment::KSeFEnvironment;
    use crate::domain::invoice::{
        Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
        LineItem, Money, Party, Quantity, VatRate,
    };
    use crate::domain::nip::Nip;
    use crate::domain::nip_account::NipAccountId;
    use crate::infra::qr::generator::QRCodeGenerator;

    fn qr_service() -> QRService {
        QRService::new(KSeFEnvironment::Test, Arc::new(QRCodeGenerator))
    }

    fn invoice_with_xml() -> Invoice {
        let nip = Nip::parse("5260250274").unwrap();
        Invoice {
            id: InvoiceId::new(),
            nip_account_id: NipAccountId::from_uuid(uuid::Uuid::from_u128(1)),
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
    fn generate_package_requires_certificate_serial_for_kod_ii() {
        let service = OfflineService::new(qr_service(), OfflineConfig::default());
        let invoice = invoice_with_xml();
        let err = service
            .generate_offline_package(&invoice, OfflineMode::Offline, " ", Utc::now())
            .unwrap_err();
        assert!(matches!(
            err,
            OfflineServiceError::Qr(QrServiceError::MissingCertificateSerial)
        ));
    }

    #[test]
    fn offline24_deadline_is_24h() {
        let service = OfflineService::new(qr_service(), OfflineConfig::default());
        let invoice = invoice_with_xml();
        let now = Utc::now();
        let pkg = service
            .generate_offline_package(&invoice, OfflineMode::Offline24, "SERIAL-1", now)
            .unwrap();
        assert_eq!(pkg.invoice.deadline_at, now + chrono::Duration::hours(24));
    }

    #[test]
    fn mark_expired_switches_status_when_deadline_passed() {
        let service = OfflineService::new(qr_service(), OfflineConfig::default());
        let invoice = invoice_with_xml();
        let now = Utc::now();
        let mut pkg = service
            .generate_offline_package(&invoice, OfflineMode::Offline24, "SERIAL-1", now)
            .unwrap();

        let changed =
            service.mark_expired_if_needed(&mut pkg.invoice, now + chrono::Duration::hours(25));
        assert!(changed);
        assert!(matches!(pkg.invoice.status, OfflineInvoiceStatus::Expired));
    }
}
