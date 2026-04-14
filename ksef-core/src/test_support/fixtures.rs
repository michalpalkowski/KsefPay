use chrono::NaiveDate;

use crate::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
    LineItem, Money, Party, PaymentMethod, Quantity, VatRate,
};
use crate::domain::nip::Nip;

/// Build a minimal valid invoice for testing.
#[must_use]
pub fn sample_invoice() -> Invoice {
    let seller_nip = Nip::parse("5260250274").unwrap();
    let buyer_nip = Nip::parse("5260250274").unwrap();

    Invoice {
        id: InvoiceId::new(),
        direction: Direction::Outgoing,
        status: InvoiceStatus::Draft,
        invoice_type: InvoiceType::Vat,
        invoice_number: "FV/2026/04/001".to_string(),
        issue_date: NaiveDate::from_ymd_opt(2026, 4, 13).unwrap(),
        sale_date: Some(NaiveDate::from_ymd_opt(2026, 4, 13).unwrap()),
        corrected_invoice_number: None,
        correction_reason: None,
        original_ksef_number: None,
        advance_payment_date: None,
        seller: Party {
            nip: Some(seller_nip),
            name: "Test Seller Sp. z o.o.".to_string(),
            address: Address {
                country_code: CountryCode::pl(),
                line1: "ul. Testowa 1".to_string(),
                line2: "00-001 Warszawa".to_string(),
            },
        },
        buyer: Party {
            nip: Some(buyer_nip),
            name: "Test Buyer S.A.".to_string(),
            address: Address {
                country_code: CountryCode::pl(),
                line1: "ul. Kupiecka 5".to_string(),
                line2: "00-002 Kraków".to_string(),
            },
        },
        currency: Currency::pln(),
        line_items: vec![LineItem {
            line_number: 1,
            description: "Usługi programistyczne".to_string(),
            unit: Some("godz".to_string()),
            quantity: Quantity::integer(160),
            unit_net_price: Some(Money::from_pln(150, 0)),
            net_value: Money::from_pln(24000, 0),
            vat_rate: VatRate::Rate23,
            vat_amount: Money::from_pln(5520, 0),
            gross_value: Money::from_pln(29520, 0),
        }],
        total_net: Money::from_pln(24000, 0),
        total_vat: Money::from_pln(5520, 0),
        total_gross: Money::from_pln(29520, 0),
        payment_method: Some(PaymentMethod::Transfer),
        payment_deadline: Some(NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()),
        bank_account: Some("PL12345678901234567890123456".to_string()),
        ksef_number: None,
        ksef_error: None,
        raw_xml: None,
    }
}
