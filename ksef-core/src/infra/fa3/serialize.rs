use std::collections::BTreeMap;
use std::fmt::Write;

use crate::domain::invoice::{Invoice, InvoiceType, Money};
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

const FA3_NAMESPACE: &str = "http://crd.gov.pl/wzor/2025/06/25/13775/";
const XSI_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
const SYSTEM_INFO: &str = "ksef-paymoney/0.1.0";

/// Serialize a domain `Invoice` into FA(3) compliant XML.
pub fn invoice_to_xml(invoice: &Invoice) -> Result<InvoiceXml, XmlError> {
    let rodzaj_faktury = map_invoice_type_to_fa3(invoice.invoice_type)?;
    let mut xml = String::with_capacity(4096);

    writeln!(xml, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        xml,
        r#"<Faktura xmlns="{FA3_NAMESPACE}" xmlns:xsi="{XSI_NAMESPACE}">"#
    )
    .unwrap();

    write_naglowek(&mut xml, invoice);
    write_podmiot1(&mut xml, invoice);
    write_podmiot2(&mut xml, invoice);
    write_fa(&mut xml, invoice, rodzaj_faktury);

    writeln!(xml, "</Faktura>").unwrap();

    Ok(InvoiceXml::new(xml))
}

fn write_naglowek(xml: &mut String, invoice: &Invoice) {
    let form_code = invoice.invoice_type.form_code();
    let kod_systemowy = format!("{form_code} (3)");

    writeln!(xml, "  <Naglowek>").unwrap();
    writeln!(
        xml,
        r#"    <KodFormularza kodSystemowy="{kod_systemowy}" wersjaSchemy="1-0E">{form_code}</KodFormularza>"#
    )
    .unwrap();
    writeln!(xml, "    <WariantFormularza>3</WariantFormularza>").unwrap();
    writeln!(
        xml,
        "    <DataWytworzeniaFa>{}T00:00:00Z</DataWytworzeniaFa>",
        invoice.issue_date
    )
    .unwrap();
    writeln!(xml, "    <SystemInfo>{SYSTEM_INFO}</SystemInfo>").unwrap();
    writeln!(xml, "  </Naglowek>").unwrap();
}

fn write_podmiot1(xml: &mut String, invoice: &Invoice) {
    let s = &invoice.seller;
    writeln!(xml, "  <Podmiot1>").unwrap();
    writeln!(xml, "    <DaneIdentyfikacyjne>").unwrap();
    if let Some(ref nip) = s.nip {
        writeln!(xml, "      <NIP>{nip}</NIP>").unwrap();
    }
    writeln!(xml, "      <Nazwa>{}</Nazwa>", escape_xml(&s.name)).unwrap();
    writeln!(xml, "    </DaneIdentyfikacyjne>").unwrap();
    writeln!(xml, "    <Adres>").unwrap();
    writeln!(xml, "      <KodKraju>{}</KodKraju>", s.address.country_code).unwrap();
    writeln!(
        xml,
        "      <AdresL1>{}</AdresL1>",
        escape_xml(&s.address.line1)
    )
    .unwrap();
    writeln!(
        xml,
        "      <AdresL2>{}</AdresL2>",
        escape_xml(&s.address.line2)
    )
    .unwrap();
    writeln!(xml, "    </Adres>").unwrap();
    writeln!(xml, "  </Podmiot1>").unwrap();
}

fn write_podmiot2(xml: &mut String, invoice: &Invoice) {
    let b = &invoice.buyer;
    writeln!(xml, "  <Podmiot2>").unwrap();
    writeln!(xml, "    <DaneIdentyfikacyjne>").unwrap();
    if let Some(ref nip) = b.nip {
        writeln!(xml, "      <NIP>{nip}</NIP>").unwrap();
    } else {
        writeln!(xml, "      <BrakID>1</BrakID>").unwrap();
    }
    writeln!(xml, "      <Nazwa>{}</Nazwa>", escape_xml(&b.name)).unwrap();
    writeln!(xml, "    </DaneIdentyfikacyjne>").unwrap();
    writeln!(xml, "    <Adres>").unwrap();
    writeln!(xml, "      <KodKraju>{}</KodKraju>", b.address.country_code).unwrap();
    writeln!(
        xml,
        "      <AdresL1>{}</AdresL1>",
        escape_xml(&b.address.line1)
    )
    .unwrap();
    writeln!(
        xml,
        "      <AdresL2>{}</AdresL2>",
        escape_xml(&b.address.line2)
    )
    .unwrap();
    writeln!(xml, "    </Adres>").unwrap();
    // Required flags in FA(3) schema v1-0:
    // 2 = "nie dotyczy" (invoice is not about JST/GV subunit handoff).
    writeln!(xml, "    <JST>2</JST>").unwrap();
    writeln!(xml, "    <GV>2</GV>").unwrap();
    writeln!(xml, "  </Podmiot2>").unwrap();
}

fn write_fa(xml: &mut String, invoice: &Invoice, rodzaj_faktury: &str) {
    writeln!(xml, "  <Fa>").unwrap();
    writeln!(xml, "    <KodWaluty>{}</KodWaluty>", invoice.currency).unwrap();
    writeln!(xml, "    <P_1>{}</P_1>", invoice.issue_date).unwrap();
    writeln!(
        xml,
        "    <P_2>{}</P_2>",
        escape_xml(&invoice.invoice_number)
    )
    .unwrap();
    if let Some(sale_date) = invoice.sale_date {
        writeln!(xml, "    <P_6>{sale_date}</P_6>").unwrap();
    }

    write_vat_summary(xml, invoice);

    writeln!(
        xml,
        "    <P_15>{}</P_15>",
        format_money(invoice.total_gross)
    )
    .unwrap();
    write_adnotacje(xml);
    writeln!(xml, "    <RodzajFaktury>{rodzaj_faktury}</RodzajFaktury>").unwrap();

    for item in &invoice.line_items {
        writeln!(xml, "    <FaWiersz>").unwrap();
        writeln!(xml, "      <NrWierszaFa>{}</NrWierszaFa>", item.line_number).unwrap();
        writeln!(xml, "      <P_7>{}</P_7>", escape_xml(&item.description)).unwrap();
        if let Some(ref unit) = item.unit {
            writeln!(xml, "      <P_8A>{}</P_8A>", escape_xml(unit)).unwrap();
        }
        writeln!(xml, "      <P_8B>{}</P_8B>", item.quantity).unwrap();
        if let Some(price) = item.unit_net_price {
            writeln!(xml, "      <P_9A>{}</P_9A>", format_money(price)).unwrap();
        }
        writeln!(xml, "      <P_11>{}</P_11>", format_money(item.net_value)).unwrap();
        writeln!(xml, "      <P_12>{}</P_12>", item.vat_rate).unwrap();
        writeln!(xml, "    </FaWiersz>").unwrap();
    }

    write_platnosc(xml, invoice);

    writeln!(xml, "  </Fa>").unwrap();
}

fn write_adnotacje(xml: &mut String) {
    writeln!(xml, "    <Adnotacje>").unwrap();
    writeln!(xml, "      <P_16>2</P_16>").unwrap();
    writeln!(xml, "      <P_17>2</P_17>").unwrap();
    writeln!(xml, "      <P_18>2</P_18>").unwrap();
    writeln!(xml, "      <P_18A>2</P_18A>").unwrap();
    writeln!(xml, "      <Zwolnienie>").unwrap();
    writeln!(xml, "        <P_19N>1</P_19N>").unwrap();
    writeln!(xml, "      </Zwolnienie>").unwrap();
    writeln!(xml, "      <NoweSrodkiTransportu>").unwrap();
    writeln!(xml, "        <P_22N>1</P_22N>").unwrap();
    writeln!(xml, "      </NoweSrodkiTransportu>").unwrap();
    writeln!(xml, "      <P_23>2</P_23>").unwrap();
    writeln!(xml, "      <PMarzy>").unwrap();
    writeln!(xml, "        <P_PMarzyN>1</P_PMarzyN>").unwrap();
    writeln!(xml, "      </PMarzy>").unwrap();
    writeln!(xml, "    </Adnotacje>").unwrap();
}

fn map_invoice_type_to_fa3(invoice_type: InvoiceType) -> Result<&'static str, XmlError> {
    match invoice_type {
        InvoiceType::Vat => Ok("VAT"),
        InvoiceType::Kor => Ok("KOR"),
        InvoiceType::Zal => Ok("ZAL"),
        InvoiceType::Roz => Ok("ROZ"),
        InvoiceType::Upr => Ok("UPR"),
        InvoiceType::KorZal => Ok("KOR_ZAL"),
        InvoiceType::KorRoz => Ok("KOR_ROZ"),
        unsupported => Err(XmlError::ValidationFailed(format!(
            "unsupported invoice type for FA(3) serializer: {unsupported}"
        ))),
    }
}

fn write_vat_summary(xml: &mut String, invoice: &Invoice) {
    // Aggregate line items by VAT rate
    let mut by_rate: BTreeMap<String, (Money, Money)> = BTreeMap::new();
    for item in &invoice.line_items {
        let key = item.vat_rate.fa3_suffix().to_string();
        let entry = by_rate
            .entry(key)
            .or_insert((Money::from_grosze(0), Money::from_grosze(0)));
        entry.0 = entry.0 + item.net_value;
        entry.1 = entry.1 + item.vat_amount;
    }

    for (suffix, (net, vat)) in &by_rate {
        writeln!(
            xml,
            "    <P_13_{suffix}>{}</P_13_{suffix}>",
            format_money(*net)
        )
        .unwrap();
        // Exempt (suffix "7") has no P_14 line
        if *suffix != "7" {
            writeln!(
                xml,
                "    <P_14_{suffix}>{}</P_14_{suffix}>",
                format_money(*vat)
            )
            .unwrap();
        }
    }
}

fn write_platnosc(xml: &mut String, invoice: &Invoice) {
    // Only write Platnosc if at least payment method or deadline is known
    if invoice.payment_method.is_none()
        && invoice.payment_deadline.is_none()
        && invoice.bank_account.is_none()
    {
        return;
    }

    writeln!(xml, "    <Platnosc>").unwrap();
    if let Some(deadline) = invoice.payment_deadline {
        writeln!(xml, "      <TerminPlatnosci>").unwrap();
        writeln!(xml, "        <Termin>{deadline}</Termin>").unwrap();
        writeln!(xml, "      </TerminPlatnosci>").unwrap();
    }
    if let Some(method) = invoice.payment_method {
        writeln!(
            xml,
            "      <FormaPlatnosci>{}</FormaPlatnosci>",
            method.fa3_code()
        )
        .unwrap();
    }

    if let Some(ref account) = invoice.bank_account {
        writeln!(xml, "      <RachunekBankowy>").unwrap();
        writeln!(xml, "        <NrRB>{account}</NrRB>").unwrap();
        writeln!(xml, "      </RachunekBankowy>").unwrap();
    }

    writeln!(xml, "    </Platnosc>").unwrap();
}

/// Format Money as "12345.67" (two decimal places, always).
fn format_money(m: Money) -> String {
    format!("{}.{:02}", m.zloty_part(), m.grosze_part().abs())
}

/// Escape XML special characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixtures::sample_invoice;
    use roxmltree::Document;

    fn validate_fa_sequence_contract(xml: &str) -> Result<(), String> {
        let doc = Document::parse(xml).map_err(|e| format!("invalid XML: {e}"))?;
        let fa = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "Fa")
            .ok_or_else(|| "missing Fa element".to_string())?;

        let mut p15_idx = None;
        let mut adnotacje_idx = None;
        let mut rodzaj_idx = None;
        let mut first_line_idx = None;

        for (idx, node) in fa.children().filter(|n| n.is_element()).enumerate() {
            match node.tag_name().name() {
                "P_15" if p15_idx.is_none() => p15_idx = Some(idx),
                "Adnotacje" if adnotacje_idx.is_none() => adnotacje_idx = Some(idx),
                "RodzajFaktury" if rodzaj_idx.is_none() => rodzaj_idx = Some(idx),
                "FaWiersz" if first_line_idx.is_none() => first_line_idx = Some(idx),
                _ => {}
            }
        }

        let p15 = p15_idx.ok_or_else(|| "missing P_15".to_string())?;
        let adnotacje = adnotacje_idx.ok_or_else(|| "missing Adnotacje".to_string())?;
        let rodzaj = rodzaj_idx.ok_or_else(|| "missing RodzajFaktury".to_string())?;
        let first_line = first_line_idx.ok_or_else(|| "missing FaWiersz".to_string())?;

        if !(p15 < adnotacje && adnotacje < rodzaj && rodzaj < first_line) {
            return Err(format!(
                "invalid Fa sequence: expected P_15 < Adnotacje < RodzajFaktury < FaWiersz, got indexes P_15={p15}, Adnotacje={adnotacje}, RodzajFaktury={rodzaj}, FaWiersz={first_line}"
            ));
        }

        Ok(())
    }

    #[test]
    fn generates_valid_fa3_xml_structure() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        // Root element with correct namespace
        assert!(s.contains(r#"<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/""#));

        // Naglowek
        assert!(s.contains(
            r#"<KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-0E">FA</KodFormularza>"#
        ));
        assert!(s.contains("<WariantFormularza>3</WariantFormularza>"));
        assert!(s.contains("<SystemInfo>ksef-paymoney/0.1.0</SystemInfo>"));

        // Podmiot1 (seller)
        assert!(s.contains("<NIP>5260250274</NIP>"));
        assert!(s.contains("<Nazwa>Test Seller Sp. z o.o.</Nazwa>"));
        assert!(s.contains("<KodKraju>PL</KodKraju>"));

        // Podmiot2 (buyer)
        assert!(s.contains("<Nazwa>Test Buyer S.A.</Nazwa>"));
        assert!(s.contains("<JST>2</JST>"));
        assert!(s.contains("<GV>2</GV>"));

        // Fa core
        assert!(s.contains("<KodWaluty>PLN</KodWaluty>"));
        assert!(s.contains("<P_1>2026-04-13</P_1>"));
        assert!(s.contains("<P_2>FV/2026/04/001</P_2>"));
        assert!(s.contains("<P_6>2026-04-13</P_6>"));
    }

    #[test]
    fn generates_vat_summary_for_23_percent() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        // 23% VAT: suffix "1"
        assert!(s.contains("<P_13_1>24000.00</P_13_1>"));
        assert!(s.contains("<P_14_1>5520.00</P_14_1>"));
    }

    #[test]
    fn generates_line_items() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(s.contains("<NrWierszaFa>1</NrWierszaFa>"));
        assert!(s.contains("<P_7>Usługi programistyczne</P_7>"));
        assert!(s.contains("<P_8A>godz</P_8A>"));
        assert!(s.contains("<P_8B>160</P_8B>"));
        assert!(s.contains("<P_9A>150.00</P_9A>"));
        assert!(s.contains("<P_11>24000.00</P_11>"));
        assert!(s.contains("<P_12>23</P_12>"));
    }

    #[test]
    fn generates_payment_section() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(s.contains("<Termin>2026-04-27</Termin>"));
        assert!(s.contains("<FormaPlatnosci>6</FormaPlatnosci>"));
        assert!(s.contains("<NrRB>PL12345678901234567890123456</NrRB>"));
    }

    #[test]
    fn generates_total() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(s.contains("<P_15>29520.00</P_15>"));
    }

    #[test]
    fn writes_required_adnotacje_and_rodzaj_faktury_before_line_items() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(s.contains("<Adnotacje>"));
        assert!(s.contains("<P_16>2</P_16>"));
        assert!(s.contains("<P_17>2</P_17>"));
        assert!(s.contains("<P_18>2</P_18>"));
        assert!(s.contains("<P_18A>2</P_18A>"));
        assert!(s.contains("<P_19N>1</P_19N>"));
        assert!(s.contains("<P_22N>1</P_22N>"));
        assert!(s.contains("<P_23>2</P_23>"));
        assert!(s.contains("<P_PMarzyN>1</P_PMarzyN>"));
        assert!(s.contains("<RodzajFaktury>VAT</RodzajFaktury>"));

        let p15_idx = s.find("<P_15>").expect("missing P_15");
        let adnotacje_idx = s.find("<Adnotacje>").expect("missing Adnotacje");
        let rodzaj_idx = s.find("<RodzajFaktury>").expect("missing RodzajFaktury");
        let line_idx = s.find("<FaWiersz>").expect("missing FaWiersz");

        assert!(p15_idx < adnotacje_idx);
        assert!(adnotacje_idx < rodzaj_idx);
        assert!(rodzaj_idx < line_idx);
        validate_fa_sequence_contract(s).unwrap();
    }

    #[test]
    fn maps_supported_invoice_types_to_rodzaj_faktury() {
        let cases = [
            (InvoiceType::Vat, "VAT"),
            (InvoiceType::Kor, "KOR"),
            (InvoiceType::Zal, "ZAL"),
            (InvoiceType::Roz, "ROZ"),
            (InvoiceType::Upr, "UPR"),
            (InvoiceType::KorZal, "KOR_ZAL"),
            (InvoiceType::KorRoz, "KOR_ROZ"),
        ];

        for (invoice_type, expected) in cases {
            let mut invoice = sample_invoice();
            invoice.invoice_type = invoice_type;
            let xml = invoice_to_xml(&invoice).unwrap();
            assert!(
                xml.as_str()
                    .contains(&format!("<RodzajFaktury>{expected}</RodzajFaktury>"))
            );
        }
    }

    #[test]
    fn rejects_invoice_types_not_supported_by_fa3_rodzaj_faktury() {
        let mut invoice = sample_invoice();
        invoice.invoice_type = InvoiceType::VatPef;

        let err = invoice_to_xml(&invoice).unwrap_err();
        assert!(matches!(err, XmlError::ValidationFailed(_)));
    }

    #[test]
    fn fa_sequence_contract_rejects_missing_adnotacje() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let broken = xml
            .as_str()
            .replace("<Adnotacje>", "")
            .replace("</Adnotacje>", "");

        let err = validate_fa_sequence_contract(&broken).unwrap_err();
        assert!(err.contains("missing Adnotacje"));
    }

    #[test]
    fn fa_sequence_contract_rejects_missing_rodzaj_faktury() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let broken = xml
            .as_str()
            .replace("<RodzajFaktury>", "")
            .replace("</RodzajFaktury>", "");

        let err = validate_fa_sequence_contract(&broken).unwrap_err();
        assert!(err.contains("missing RodzajFaktury"));
    }

    #[test]
    fn fa_sequence_contract_rejects_fawiersz_before_adnotacje() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).unwrap();
        let injected_early_line =
            r#"<FaWiersz><NrWierszaFa>999</NrWierszaFa><P_7>X</P_7></FaWiersz><Adnotacje>"#;
        let broken = xml.as_str().replacen("<Adnotacje>", injected_early_line, 1);

        let err = validate_fa_sequence_contract(&broken).unwrap_err();
        assert!(err.contains("invalid Fa sequence"));
    }

    #[test]
    fn escapes_xml_special_chars_in_name() {
        let mut invoice = sample_invoice();
        invoice.seller.name = "Firma & Synowie <test>".to_string();
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(s.contains("<Nazwa>Firma &amp; Synowie &lt;test&gt;</Nazwa>"));
        assert!(!s.contains("<Nazwa>Firma & Synowie <test></Nazwa>"));
    }

    #[test]
    fn omits_bank_account_when_none() {
        let mut invoice = sample_invoice();
        invoice.bank_account = None;
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(!s.contains("<RachunekBankowy>"));
        assert!(!s.contains("<NrRB>"));
    }

    #[test]
    fn omits_unit_when_none() {
        let mut invoice = sample_invoice();
        invoice.line_items[0].unit = None;
        let xml = invoice_to_xml(&invoice).unwrap();
        let s = xml.as_str();

        assert!(!s.contains("<P_8A>"));
    }

    #[test]
    fn format_money_works() {
        assert_eq!(format_money(Money::from_grosze(12345)), "123.45");
        assert_eq!(format_money(Money::from_grosze(100)), "1.00");
        assert_eq!(format_money(Money::from_grosze(5)), "0.05");
        assert_eq!(format_money(Money::from_grosze(0)), "0.00");
    }
}
