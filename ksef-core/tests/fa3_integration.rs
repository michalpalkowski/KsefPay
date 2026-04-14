use ksef_core::domain::invoice::{Direction, Money, PaymentMethod};
use ksef_core::domain::session::KSeFNumber;
use ksef_core::domain::xml::{InvoiceXml, UntrustedInvoiceXml};
use ksef_core::infra::fa3::xml_to_invoice;

#[test]
fn fa3_parser_accepts_all_supported_payment_method_codes() {
    let cases = [
        (1, PaymentMethod::Cash),
        (2, PaymentMethod::Card),
        (3, PaymentMethod::Voucher),
        (4, PaymentMethod::Check),
        (5, PaymentMethod::Credit),
        (6, PaymentMethod::Transfer),
        (7, PaymentMethod::Mobile),
    ];

    for (code, expected_method) in cases {
        let xml = build_minimal_xml(
            r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11>100.00</P_11><P_12>23</P_12></FaWiersz>"#,
            code,
            "123.00",
        );

        let trusted = InvoiceXml::from_untrusted(UntrustedInvoiceXml::new(xml)).unwrap();
        let invoice = xml_to_invoice(
            &trusted,
            Direction::Incoming,
            &KSeFNumber::new("KSeF-INT-001".to_string()),
        )
        .unwrap();

        assert_eq!(invoice.payment_method, Some(expected_method));
        assert_eq!(invoice.payment_method.unwrap().fa3_code(), code);
    }
}

#[test]
fn fa3_parser_accepts_p11a_as_line_net_value_fallback() {
    let xml = build_minimal_xml(
        r#"<FaWiersz><NrWierszaFa>1</NrWierszaFa><P_7>x</P_7><P_8B>1</P_8B><P_9A>100.00</P_9A><P_11A>100.00</P_11A><P_12>23</P_12></FaWiersz>"#,
        6,
        "123.00",
    );

    let trusted = InvoiceXml::from_untrusted(UntrustedInvoiceXml::new(xml)).unwrap();
    let invoice = xml_to_invoice(
        &trusted,
        Direction::Incoming,
        &KSeFNumber::new("KSeF-INT-002".to_string()),
    )
    .unwrap();

    assert_eq!(invoice.line_items[0].net_value, Money::from_grosze(10_000));
}

fn build_minimal_xml(fa_wiersze: &str, payment_code: u8, total_gross: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/">
  <Naglowek>
    <KodFormularza kodSystemowy="FA (3)" wersjaSchemy="1-0E">FA</KodFormularza>
    <WariantFormularza>3</WariantFormularza>
    <DataWytworzeniaFa>2026-04-13T00:00:00Z</DataWytworzeniaFa>
    <SystemInfo>integration-test</SystemInfo>
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
    <Platnosc>
      <TerminPlatnosci><Termin>2026-04-27</Termin></TerminPlatnosci>
      <FormaPlatnosci>{payment_code}</FormaPlatnosci>
    </Platnosc>
  </Fa>
</Faktura>"#
    )
}
