use std::fs;
use std::path::PathBuf;

use chrono::NaiveDate;
use ksef_core::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
    LineItem, Money, Party, PaymentMethod, Quantity, VatRate,
};
use ksef_core::domain::nip::Nip;
use ksef_core::domain::nip_account::NipAccountId;
use ksef_core::infra::fa3::invoice_to_xml;
use uppsala::xsd::XsdValidator;

fn schema_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join("fa3")
        .join("2025-06-25-13775")
        .join(file)
}

fn validate_against_official_fa3_xsd(xml: &str) -> Vec<String> {
    let schema_file = schema_path("schemat.xsd");
    let schema_xml = fs::read_to_string(&schema_file).expect("failed to read schemat.xsd");
    let schema_doc = uppsala::parse(&schema_xml).expect("failed to parse schemat.xsd");
    let validator = XsdValidator::from_schema_with_base_path(&schema_doc, Some(&schema_file))
        .expect("failed to compile official FA(3) XSD bundle");

    let instance_doc = uppsala::parse(xml).expect("failed to parse instance XML");
    validator
        .validate(&instance_doc)
        .into_iter()
        .map(|e| e.message)
        .collect()
}

fn sample_invoice() -> Invoice {
    let seller_nip = Nip::parse("5260250274").unwrap();
    let buyer_nip = Nip::parse("5260250274").unwrap();

    Invoice {
        id: InvoiceId::new(),
        nip_account_id: NipAccountId::from_uuid(uuid::Uuid::from_u128(1)),
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
                line2: "00-002 Krakow".to_string(),
            },
        },
        currency: Currency::pln(),
        line_items: vec![LineItem {
            line_number: 1,
            description: "Uslugi programistyczne".to_string(),
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

#[test]
fn generated_xml_validates_against_official_fa3_xsd_bundle() {
    let invoice = sample_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();

    let errors = validate_against_official_fa3_xsd(xml.as_str());
    assert!(
        errors.is_empty(),
        "expected valid XML against official FA(3) XSD, got: {errors:#?}"
    );
}

#[test]
fn missing_required_rodzaj_faktury_is_rejected_by_official_xsd() {
    let invoice = sample_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();
    let broken = xml
        .as_str()
        .replace("<RodzajFaktury>VAT</RodzajFaktury>\n", "");

    let errors = validate_against_official_fa3_xsd(&broken);
    assert!(
        !errors.is_empty(),
        "expected missing RodzajFaktury to fail XSD validation"
    );
}

#[test]
fn invalid_fa_order_is_rejected_by_official_xsd() {
    let invoice = sample_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();
    let broken = xml.as_str().replacen(
        "<Adnotacje>",
        "<FaWiersz><NrWierszaFa>999</NrWierszaFa><P_7>X</P_7><P_8B>1</P_8B><P_11>1.00</P_11><P_12>23</P_12></FaWiersz><Adnotacje>",
        1,
    );

    let errors = validate_against_official_fa3_xsd(&broken);
    assert!(
        !errors.is_empty(),
        "expected invalid element order to fail XSD validation"
    );
}
