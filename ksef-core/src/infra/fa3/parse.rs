use chrono::NaiveDate;
use roxmltree::{Document, Node};

use crate::domain::invoice::{
    Address, CountryCode, Currency, Direction, FormCode, Invoice, InvoiceId, InvoiceStatus,
    InvoiceType, LineItem, Money, Party, PaymentMethod, Quantity, VatRate,
};
use crate::domain::nip::Nip;
use crate::domain::nip_account::NipAccountId;
use crate::domain::session::KSeFNumber;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

/// Parse a FA(3) XML document into a domain `Invoice`.
///
/// Strict parser: missing required elements produce `XmlError`, not defaults.
/// Optional elements (`P_8A`, `RachunekBankowy`) are represented as `Option`.
pub fn xml_to_invoice(
    xml: &InvoiceXml,
    direction: Direction,
    ksef_number: &KSeFNumber,
) -> Result<Invoice, XmlError> {
    let doc = Document::parse(xml.as_str())
        .map_err(|e| XmlError::ParseFailed(format!("invalid XML: {e}")))?;

    let root = doc.root_element();
    if root.tag_name().name() != "Faktura" {
        return Err(XmlError::ParseFailed(format!(
            "expected root element 'Faktura', got '{}'",
            root.tag_name().name()
        )));
    }

    let naglowek = required_child(&root, "Naglowek")?;
    validate_schema_version(&naglowek)?;
    let invoice_type = parse_invoice_type(&naglowek)?;

    let podmiot1 = required_child(&root, "Podmiot1")?;
    let seller = parse_party(&podmiot1)?;

    let podmiot2 = required_child(&root, "Podmiot2")?;
    let buyer = parse_party(&podmiot2)?;

    let fa = required_child(&root, "Fa")?;
    let currency = parse_currency(&fa)?;
    let invoice_number = required_text(&fa, "P_2")?;
    let issue_date = parse_date_element(&fa, "P_1")?;
    let sale_date = optional_date_element(&fa, "P_6")?;
    let total_gross = parse_money_element(&fa, "P_15")?;

    let line_items = parse_line_items_optional(&fa)?;
    let (total_net, total_vat) = compute_totals_from_items(&line_items);

    let (payment_method, payment_deadline, bank_account) = parse_platnosc_optional(&fa)?;

    Ok(Invoice {
        id: InvoiceId::new(),
        // Caller is responsible for assigning the real tenant account before persistence.
        nip_account_id: NipAccountId::new(),
        direction,
        status: InvoiceStatus::Fetched,
        invoice_type,
        invoice_number,
        issue_date,
        sale_date,
        corrected_invoice_number: None,
        correction_reason: None,
        original_ksef_number: None,
        advance_payment_date: None,
        seller,
        buyer,
        currency,
        line_items,
        total_net,
        total_vat,
        total_gross,
        payment_method,
        payment_deadline,
        bank_account,
        ksef_number: Some(ksef_number.clone()),
        ksef_error: None,
        raw_xml: Some(xml.as_str().to_string()),
    })
}

// --- Internal parsing helpers ---

fn required_child<'a>(parent: &'a Node, name: &str) -> Result<Node<'a, 'a>, XmlError> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .ok_or_else(|| XmlError::MissingElement(name.to_string()))
}

fn optional_child<'a>(parent: &'a Node, name: &str) -> Option<Node<'a, 'a>> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

fn required_text(parent: &Node, element_name: &str) -> Result<String, XmlError> {
    let child = required_child(parent, element_name)?;
    child
        .text()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| XmlError::MissingElement(format!("{element_name} (empty text)")))
}

fn optional_text(parent: &Node, element_name: &str) -> Option<String> {
    optional_child(parent, element_name)
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn validate_schema_version(naglowek: &Node) -> Result<(), XmlError> {
    let kod = required_child(naglowek, "KodFormularza")?;

    // Verify that wersjaSchemy attribute exists and looks like an FA(3) version.
    // We don't reject unknown versions — KSeF may issue newer revisions (1-0E, 1-1E, ...)
    // and the XML structure stays backward compatible. We validate the actual content,
    // not the version number.
    let _wersja = kod
        .attribute("wersjaSchemy")
        .ok_or_else(|| XmlError::MissingElement("KodFormularza@wersjaSchemy".to_string()))?;

    Ok(())
}

fn parse_invoice_type(naglowek: &Node) -> Result<InvoiceType, XmlError> {
    let form_code_text = required_text(naglowek, "KodFormularza")?;
    let form_code: FormCode = form_code_text.parse().map_err(|_| XmlError::InvalidValue {
        element: "KodFormularza".to_string(),
        reason: format!("unsupported form code: '{form_code_text}'"),
    })?;

    Ok(match form_code {
        FormCode::Fa => InvoiceType::Vat,
        FormCode::Kor => InvoiceType::Kor,
        FormCode::Zal => InvoiceType::Zal,
        FormCode::Roz => InvoiceType::Roz,
        FormCode::Upr => InvoiceType::Upr,
        FormCode::VatPef => InvoiceType::VatPef,
        FormCode::VatPefSp => InvoiceType::VatPefSp,
        FormCode::KorPef => InvoiceType::KorPef,
        FormCode::VatRr => InvoiceType::VatRr,
        FormCode::KorVatRr => InvoiceType::KorVatRr,
        FormCode::KorZal => InvoiceType::KorZal,
        FormCode::KorRoz => InvoiceType::KorRoz,
    })
}

fn parse_date_element(parent: &Node, element_name: &str) -> Result<NaiveDate, XmlError> {
    let raw = required_text(parent, element_name)?;
    parse_date_string(&raw, element_name)
}

fn parse_date_string(raw: &str, element_name: &str) -> Result<NaiveDate, XmlError> {
    // Try plain date first, then extract date prefix from datetime
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .or_else(|_| {
            raw.get(0..10)
                .and_then(|prefix| NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok())
                .ok_or(())
        })
        .map_err(|()| XmlError::InvalidValue {
            element: element_name.to_string(),
            reason: format!("cannot parse date: '{raw}'"),
        })
}

fn optional_date_element(parent: &Node, element_name: &str) -> Result<Option<NaiveDate>, XmlError> {
    match optional_text(parent, element_name) {
        Some(raw) => parse_date_string(&raw, element_name).map(Some),
        None => Ok(None),
    }
}

fn optional_money_element(parent: &Node, element_name: &str) -> Result<Option<Money>, XmlError> {
    match optional_text(parent, element_name) {
        Some(raw) => {
            let m = raw.parse::<Money>().map_err(|e| XmlError::InvalidValue {
                element: element_name.to_string(),
                reason: format!("invalid money amount: {e}"),
            })?;
            Ok(Some(m))
        }
        None => Ok(None),
    }
}

fn parse_money_element(parent: &Node, element_name: &str) -> Result<Money, XmlError> {
    let raw = required_text(parent, element_name)?;
    raw.parse::<Money>().map_err(|e| XmlError::InvalidValue {
        element: element_name.to_string(),
        reason: format!("invalid money amount: {e}"),
    })
}

fn parse_line_net_value(row: &Node) -> Result<Money, XmlError> {
    if let Some(value) = optional_money_element(row, "P_11")? {
        return Ok(value);
    }

    if let Some(value) = optional_money_element(row, "P_11A")? {
        return Ok(value);
    }

    Err(XmlError::MissingElement("P_11".to_string()))
}

fn parse_party(podmiot: &Node) -> Result<Party, XmlError> {
    let dane = required_child(podmiot, "DaneIdentyfikacyjne")?;

    // NIP is optional — buyers may be individuals (BrakID) or foreign entities.
    let nip = match optional_text(&dane, "NIP") {
        Some(nip_str) => Some(Nip::parse(&nip_str).map_err(|e| XmlError::InvalidValue {
            element: "NIP".to_string(),
            reason: format!("'{nip_str}': {e}"),
        })?),
        None => None,
    };

    let name = required_text(&dane, "Nazwa")?;

    let adres = required_child(podmiot, "Adres")?;
    let country_str = required_text(&adres, "KodKraju")?;
    let country_code = CountryCode::parse(&country_str).map_err(|e| XmlError::InvalidValue {
        element: "KodKraju".to_string(),
        reason: format!("{e}"),
    })?;
    let line1 = required_text(&adres, "AdresL1")?;
    let line2 = optional_text(&adres, "AdresL2").unwrap_or_default();

    Ok(Party {
        nip,
        name,
        address: Address {
            country_code,
            line1,
            line2,
        },
    })
}

fn parse_currency(fa: &Node) -> Result<Currency, XmlError> {
    let raw = required_text(fa, "KodWaluty")?;
    Currency::parse(&raw).map_err(|e| XmlError::InvalidValue {
        element: "KodWaluty".to_string(),
        reason: format!("{e}"),
    })
}

fn parse_line_items_optional(fa: &Node) -> Result<Vec<LineItem>, XmlError> {
    let rows: Vec<Node> = fa
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "FaWiersz")
        .collect();

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        items.push(parse_line_item(row)?);
    }
    Ok(items)
}

fn parse_line_item(row: &Node) -> Result<LineItem, XmlError> {
    let line_number_str = required_text(row, "NrWierszaFa")?;
    let line_number: u32 = line_number_str
        .parse()
        .map_err(|_| XmlError::InvalidValue {
            element: "NrWierszaFa".to_string(),
            reason: format!("not a valid line number: '{line_number_str}'"),
        })?;

    let description = required_text(row, "P_7")?;
    let unit = optional_text(row, "P_8A");

    let quantity_str = required_text(row, "P_8B")?;
    let quantity = Quantity::parse(&quantity_str).map_err(|e| XmlError::InvalidValue {
        element: "P_8B".to_string(),
        reason: format!("{e}"),
    })?;

    let unit_net_price = optional_money_element(row, "P_9A")?;
    let net_value = parse_line_net_value(row)?;

    let vat_rate_str = required_text(row, "P_12")?;
    let vat_rate: VatRate = vat_rate_str.parse().map_err(|e| XmlError::InvalidValue {
        element: "P_12".to_string(),
        reason: format!("{e}"),
    })?;

    // Compute VAT amount and gross from net + rate
    let vat_grosze = match vat_rate.percentage() {
        Some(pct) => {
            let numerator = i128::from(net_value.grosze()) * i128::from(pct);
            let rounded = div_round_half_away_from_zero(numerator, 100);
            i64::try_from(rounded).map_err(|_| XmlError::InvalidValue {
                element: "P_11".to_string(),
                reason: "VAT amount overflow".to_string(),
            })?
        }
        None => 0,
    };
    let vat_amount = Money::from_grosze(vat_grosze);
    let gross_value = Money::from_grosze(net_value.grosze() + vat_grosze);

    Ok(LineItem {
        line_number,
        description,
        unit,
        quantity,
        unit_net_price,
        net_value,
        vat_rate,
        vat_amount,
        gross_value,
    })
}

type PaymentDetails = (Option<PaymentMethod>, Option<NaiveDate>, Option<String>);

fn parse_platnosc_optional(fa: &Node) -> Result<PaymentDetails, XmlError> {
    let Some(platnosc) = optional_child(fa, "Platnosc") else {
        return Ok((None, None, None));
    };

    let payment_deadline = match optional_child(&platnosc, "TerminPlatnosci")
        .and_then(|t| optional_text(&t, "Termin"))
    {
        Some(raw) => Some(parse_date_string(&raw, "Termin")?),
        None => None,
    };

    let payment_method = match optional_text(&platnosc, "FormaPlatnosci") {
        Some(code) => {
            let parsed_code = code.parse::<u8>().map_err(|_| XmlError::InvalidValue {
                element: "FormaPlatnosci".to_string(),
                reason: format!("unsupported payment method code: '{code}'"),
            })?;
            let method =
                PaymentMethod::try_from(parsed_code).map_err(|_| XmlError::InvalidValue {
                    element: "FormaPlatnosci".to_string(),
                    reason: format!("unsupported payment method code: '{code}'"),
                })?;
            Some(method)
        }
        None => None,
    };

    let bank_account =
        optional_child(&platnosc, "RachunekBankowy").and_then(|rb| optional_text(&rb, "NrRB"));

    Ok((payment_method, payment_deadline, bank_account))
}

fn compute_totals_from_items(items: &[LineItem]) -> (Money, Money) {
    let total_net = items
        .iter()
        .fold(Money::from_grosze(0), |acc, item| acc + item.net_value);
    let total_vat = items
        .iter()
        .fold(Money::from_grosze(0), |acc, item| acc + item.vat_amount);
    (total_net, total_vat)
}

fn div_round_half_away_from_zero(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder == 0 {
        return quotient;
    }
    if remainder.abs() * 2 >= denominator.abs() {
        quotient + numerator.signum()
    } else {
        quotient
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fa3::invoice_to_xml;
    use crate::test_support::fixtures::sample_invoice;

    // --- Round-trip test ---

    #[test]
    fn round_trip_serialize_then_parse() {
        let original = sample_invoice();
        let xml = invoice_to_xml(&original).unwrap();
        let ksef_num = KSeFNumber::new("KSeF-TEST-001".to_string());

        let parsed = xml_to_invoice(&xml, Direction::Outgoing, &ksef_num).unwrap();

        assert_eq!(parsed.direction, Direction::Outgoing);
        assert_eq!(parsed.status, InvoiceStatus::Fetched);
        assert_eq!(parsed.invoice_type, original.invoice_type);
        assert_eq!(parsed.invoice_number, original.invoice_number);
        assert_eq!(parsed.issue_date, original.issue_date);
        assert_eq!(parsed.sale_date, original.sale_date);
        assert_eq!(parsed.seller.nip, original.seller.nip);
        assert_eq!(parsed.seller.name, original.seller.name);
        assert_eq!(parsed.buyer.nip, original.buyer.nip);
        assert_eq!(parsed.buyer.name, original.buyer.name);
        assert_eq!(parsed.currency.as_str(), original.currency.as_str());
        assert_eq!(parsed.total_gross, original.total_gross);
        assert_eq!(parsed.total_net, original.total_net);
        assert_eq!(parsed.total_vat, original.total_vat);
        assert_eq!(parsed.line_items.len(), original.line_items.len());

        let item = &parsed.line_items[0];
        let orig_item = &original.line_items[0];
        assert_eq!(item.line_number, orig_item.line_number);
        assert_eq!(item.description, orig_item.description);
        assert_eq!(item.unit, orig_item.unit);
        assert_eq!(item.quantity.to_string(), orig_item.quantity.to_string());
        assert_eq!(item.unit_net_price, orig_item.unit_net_price);
        assert_eq!(item.net_value, orig_item.net_value);
        assert_eq!(item.vat_rate, orig_item.vat_rate);
        assert_eq!(item.vat_amount, orig_item.vat_amount);
        assert_eq!(item.gross_value, orig_item.gross_value);

        assert_eq!(parsed.payment_method, original.payment_method);
        assert_eq!(parsed.payment_deadline, original.payment_deadline);
        assert_eq!(parsed.bank_account, original.bank_account);
        assert_eq!(parsed.ksef_number.unwrap().as_str(), "KSeF-TEST-001");
        assert!(parsed.raw_xml.is_some());
    }

    // --- Naglowek ---

    #[test]
    fn accepts_unknown_schema_version() {
        // Parser should not reject unknown schema versions — KSeF may issue newer
        // revisions and the XML structure stays backward compatible.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek>
    <KodFormularza kodSystemowy="FA (3)" wersjaSchemy="2-0E">FA</KodFormularza>
    <WariantFormularza>3</WariantFormularza>
    <DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa>
    <SystemInfo>test</SystemInfo>
  </Naglowek>
  <Podmiot1><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>S</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>a</AdresL1><AdresL2>b</AdresL2></Adres></Podmiot1>
  <Podmiot2><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>B</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>c</AdresL1><AdresL2>d</AdresL2></Adres></Podmiot2>
  <Fa><KodWaluty>PLN</KodWaluty><P_1>2026-04-13</P_1><P_2>FV/1</P_2><P_6>2026-04-13</P_6><P_15>123.00</P_15>
  <FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>
  <Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>6</FormaPlatnosci></Platnosc></Fa>
</Faktura>"#;

        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.invoice_number, "FV/1");
    }

    #[test]
    fn rejects_missing_schema_version_attribute() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek>
    <KodFormularza kodSystemowy="FA (3)">FA</KodFormularza>
    <WariantFormularza>3</WariantFormularza>
    <DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa>
    <SystemInfo>test</SystemInfo>
  </Naglowek>
  <Podmiot1><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>S</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>a</AdresL1><AdresL2>b</AdresL2></Adres></Podmiot1>
  <Podmiot2><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>B</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>c</AdresL1><AdresL2>d</AdresL2></Adres></Podmiot2>
  <Fa><KodWaluty>PLN</KodWaluty><P_1>2026-04-13</P_1><P_2>FV/1</P_2><P_6>2026-04-13</P_6><P_15>100.00</P_15>
  <FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>
  <Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>6</FormaPlatnosci></Platnosc></Fa>
</Faktura>"#;

        let err = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::MissingElement(ref e) if e.contains("wersjaSchemy")));
    }

    #[test]
    fn accepts_schema_version_1_1e() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek>
    <KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-1E">FA</KodFormularza>
    <WariantFormularza>3</WariantFormularza>
    <DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa>
    <SystemInfo>test</SystemInfo>
  </Naglowek>
  <Podmiot1><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>Seller</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>ul. A 1</AdresL1><AdresL2>00-001 W</AdresL2></Adres></Podmiot1>
  <Podmiot2><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>Buyer</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>ul. B 2</AdresL1><AdresL2>00-002 K</AdresL2></Adres></Podmiot2>
  <Fa><KodWaluty>PLN</KodWaluty><P_1>2026-04-13</P_1><P_2>FV/1</P_2><P_6>2026-04-13</P_6><P_15>100.00</P_15>
  <FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>Uslugi</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>
  <Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>6</FormaPlatnosci></Platnosc></Fa>
</Faktura>"#;

        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-1-1E".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.invoice_number, "FV/1");
    }

    // --- Missing elements ---

    #[test]
    fn rejects_missing_podmiot1() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek>
    <KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-0E">FA</KodFormularza>
    <WariantFormularza>3</WariantFormularza>
    <DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa>
    <SystemInfo>test</SystemInfo>
  </Naglowek>
</Faktura>"#;

        let err = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::MissingElement(ref e) if e == "Podmiot1"));
    }

    #[test]
    fn parses_invoice_without_line_items() {
        let xml = build_minimal_xml("", "0.00");
        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert!(invoice.line_items.is_empty());
        assert_eq!(invoice.total_net, Money::from_grosze(0));
    }

    // --- Invalid values ---

    #[test]
    fn rejects_invalid_nip_in_podmiot() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek><KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-0E">FA</KodFormularza><WariantFormularza>3</WariantFormularza><DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa><SystemInfo>t</SystemInfo></Naglowek>
  <Podmiot1><DaneIdentyfikacyjne><NIP>1234567890</NIP><Nazwa>S</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>a</AdresL1><AdresL2>b</AdresL2></Adres></Podmiot1>
  <Podmiot2><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>B</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>c</AdresL1><AdresL2>d</AdresL2></Adres></Podmiot2>
  <Fa><KodWaluty>PLN</KodWaluty><P_1>2026-04-13</P_1><P_2>FV/1</P_2><P_6>2026-04-13</P_6><P_15>123.00</P_15>
  <FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>
  <Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>6</FormaPlatnosci></Platnosc></Fa>
</Faktura>"#;

        let err = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::InvalidValue { ref element, .. } if element == "NIP"));
    }

    #[test]
    fn rejects_invalid_vat_rate() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>99</P_12></FaWiersz>"#;
        let xml = build_minimal_xml(wiersz, "199.00");
        let err = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::InvalidValue { ref element, .. } if element == "P_12"));
    }

    #[test]
    fn accepts_qualified_reverse_charge_vat_rate() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>np I</P_12></FaWiersz>"#;
        let xml = build_minimal_xml(wiersz, "100.00");
        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.line_items[0].vat_rate, VatRate::ReverseCharge);
        assert_eq!(invoice.line_items[0].vat_amount, Money::from_grosze(0));
    }

    #[test]
    fn rejects_invalid_optional_sale_date_when_present() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek><KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-0E">FA</KodFormularza><WariantFormularza>3</WariantFormularza><DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa><SystemInfo>t</SystemInfo></Naglowek>
  <Podmiot1><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>S</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>a</AdresL1><AdresL2>b</AdresL2></Adres></Podmiot1>
  <Podmiot2><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>B</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>c</AdresL1><AdresL2>d</AdresL2></Adres></Podmiot2>
  <Fa><KodWaluty>PLN</KodWaluty><P_1>2026-04-13</P_1><P_2>FV/1</P_2><P_6>2026-99-99</P_6><P_15>123.00</P_15>
  <FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>
  </Fa>
</Faktura>"#;

        let err = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::InvalidValue { ref element, .. } if element == "P_6"));
    }

    #[test]
    fn rejects_missing_required_line_net_value() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_12>23</P_12></FaWiersz>"#;
        let xml = build_minimal_xml(wiersz, "123.00");
        let err = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::MissingElement(ref e) if e == "P_11"));
    }

    #[test]
    fn accepts_line_net_value_from_p11a_when_p11_missing() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11A>100.00</P_11A><P_12>23</P_12></FaWiersz>"#;
        let xml = build_minimal_xml(wiersz, "123.00");
        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.line_items[0].net_value, Money::from_grosze(10_000));
    }

    #[test]
    fn accepts_p15_different_from_computed_gross() {
        // KSeF is source of truth — P_15 may differ from computed due to rounding/corrections
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>"#;
        let xml = build_minimal_xml(wiersz, "999.00");
        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.total_gross, Money::from_grosze(99900));
    }

    #[test]
    fn unknown_payment_method_returns_error() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>"#;
        let platnosc = r#"<Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>99</FormaPlatnosci></Platnosc>"#;
        let xml = build_minimal_xml_with_platnosc(wiersz, platnosc, "123.00");
        let err = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            XmlError::InvalidValue {
                ref element,
                ref reason
            } if element == "FormaPlatnosci"
                && reason.contains("unsupported payment method code")
        ));
    }

    #[test]
    fn accepts_mobile_payment_method_code_7() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>"#;
        let platnosc = r#"<Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>7</FormaPlatnosci></Platnosc>"#;
        let xml = build_minimal_xml_with_platnosc(wiersz, platnosc, "123.00");
        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.payment_method, Some(PaymentMethod::Mobile));
    }

    // --- Optional fields ---

    #[test]
    fn parses_without_unit_and_bank_account() {
        let wiersz = r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>Service</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>"#;
        let platnosc = r#"<Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>1</FormaPlatnosci></Platnosc>"#;
        let xml = build_minimal_xml_with_platnosc(wiersz, platnosc, "123.00");

        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert!(invoice.line_items[0].unit.is_none());
        assert!(invoice.bank_account.is_none());
        assert_eq!(invoice.payment_method, Some(PaymentMethod::Cash));
    }

    // --- Multiple VAT rates ---

    #[test]
    fn parses_multiple_vat_rates() {
        let wiersze = r#"
        <FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>Item23</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>
        <FaWiersz><NrWierszaFa>2</NrWierszaFa><P_7>Item8</P_7><P_8B>2</P_8B><P_9A>50.00</P_9A><P_11>100.00</P_11><P_12>8</P_12></FaWiersz>
        <FaWiersz><NrWierszaFa>3</NrWierszaFa><P_7>ItemZW</P_7><P_8B>1</P_8B><P_9A>200.00</P_9A><P_11>200.00</P_11><P_12>zw</P_12></FaWiersz>"#;
        // net: 100+100+200=400, vat: 23+8+0=31, gross: 431
        let xml = build_minimal_xml(wiersze, "431.00");
        let invoice = xml_to_invoice(
            &InvoiceXml::new(xml),
            Direction::Incoming,
            &KSeFNumber::new("KSeF-X".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.line_items.len(), 3);
        assert_eq!(invoice.line_items[0].vat_rate, VatRate::Rate23);
        assert_eq!(invoice.line_items[1].vat_rate, VatRate::Rate8);
        assert_eq!(invoice.line_items[2].vat_rate, VatRate::Exempt);
        assert_eq!(invoice.line_items[2].vat_amount, Money::from_grosze(0));
    }

    // --- Status and direction ---

    #[test]
    fn parsed_invoice_has_fetched_status() {
        let original = sample_invoice();
        let xml = invoice_to_xml(&original).unwrap();
        let ksef_num = KSeFNumber::new("KSeF-TEST-002".to_string());

        let parsed = xml_to_invoice(&xml, Direction::Incoming, &ksef_num).unwrap();
        assert_eq!(parsed.status, InvoiceStatus::Fetched);
    }

    #[test]
    fn parsed_invoice_uses_provided_direction() {
        let original = sample_invoice();
        let xml = invoice_to_xml(&original).unwrap();
        let ksef_num = KSeFNumber::new("KSeF-TEST-003".to_string());

        let outgoing = xml_to_invoice(&xml, Direction::Outgoing, &ksef_num).unwrap();
        assert_eq!(outgoing.direction, Direction::Outgoing);

        let incoming = xml_to_invoice(&xml, Direction::Incoming, &ksef_num).unwrap();
        assert_eq!(incoming.direction, Direction::Incoming);
    }

    // --- Malformed XML ---

    #[test]
    fn rejects_malformed_xml() {
        let err = xml_to_invoice(
            &InvoiceXml::new("<not valid xml".to_string()),
            Direction::Incoming,
            &KSeFNumber::new("X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::ParseFailed(_)));
    }

    #[test]
    fn rejects_wrong_root_element() {
        let xml = r#"<?xml version="1.0"?><NotFaktura/>"#;
        let err = xml_to_invoice(
            &InvoiceXml::new(xml.to_string()),
            Direction::Incoming,
            &KSeFNumber::new("X".to_string()),
        )
        .unwrap_err();

        assert!(matches!(err, XmlError::ParseFailed(ref msg) if msg.contains("NotFaktura")));
    }

    // --- Test helpers ---

    fn build_minimal_xml(fa_wiersze: &str, total_gross: &str) -> String {
        let platnosc = r#"<Platnosc><TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci><FormaPlatnosci>6</FormaPlatnosci></Platnosc>"#;
        build_minimal_xml_full(fa_wiersze, platnosc, total_gross)
    }

    fn build_minimal_xml_with_platnosc(
        fa_wiersze: &str,
        platnosc: &str,
        total_gross: &str,
    ) -> String {
        build_minimal_xml_full(fa_wiersze, platnosc, total_gross)
    }

    fn build_minimal_xml_full(fa_wiersze: &str, platnosc: &str, total_gross: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek>
    <KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-0E">FA</KodFormularza>
    <WariantFormularza>3</WariantFormularza>
    <DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa>
    <SystemInfo>test</SystemInfo>
  </Naglowek>
  <Podmiot1><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>Seller</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>ul. A</AdresL1><AdresL2>00-001</AdresL2></Adres></Podmiot1>
  <Podmiot2><DaneIdentyfikacyjne><NIP>5260250274</NIP><Nazwa>Buyer</Nazwa></DaneIdentyfikacyjne><Adres><KodKraju>PL</KodKraju><AdresL1>ul. B</AdresL1><AdresL2>00-002</AdresL2></Adres></Podmiot2>
  <Fa>
    <KodWaluty>PLN</KodWaluty>
    <P_1>2026-04-13</P_1>
    <P_2>FV/TEST/001</P_2>
    <P_6>2026-04-13</P_6>
    <P_15>{total_gross}</P_15>
    {fa_wiersze}
    {platnosc}
  </Fa>
</Faktura>"#
        )
    }
}
