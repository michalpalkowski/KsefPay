use std::collections::BTreeMap;

use chrono::NaiveDate;
use xsd_parser_types::quick_xml as qx;
use xsd_parser_types::quick_xml::{
    DeserializeBytes, DeserializeSync, SerializeBytes, SerializeSync,
};

use crate::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
    LineItem, Money, Party, PaymentMethod, Quantity, VatRate,
};
use crate::domain::nip::Nip;
use crate::domain::nip_account::NipAccountId;
use crate::domain::session::KSeFNumber;
use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;

use super::generated::v2025_06_25_13775 as fa3;

const SYSTEM_INFO: &str = "ksef-paymoney/0.1.0";
const SCHEMA_VERSION: &str = "1-0E";

type FaContent = fa3::tns::FakturaFaElementTypeContent;

pub fn invoice_to_xml(invoice: &Invoice) -> Result<InvoiceXml, XmlError> {
    let model = domain_to_model(invoice)?;

    let mut buffer = Vec::with_capacity(4096);
    let mut writer = qx::Writer::new_with_indent(&mut buffer, b' ', 2);
    writer
        .write_event(qx::Event::Decl(qx::BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            None,
        )))
        .map_err(|e| XmlError::SerializationFailed(format!("cannot write XML declaration: {e}")))?;

    model
        .serialize("tns:Faktura", &mut writer)
        .map_err(|e| XmlError::SerializationFailed(format!("cannot serialize FA(3) XML: {e}")))?;

    let xml = String::from_utf8(buffer)
        .map_err(|e| XmlError::SerializationFailed(format!("invalid UTF-8 XML output: {e}")))?;

    Ok(InvoiceXml::new(xml))
}

pub fn xml_to_invoice(
    xml: &InvoiceXml,
    direction: Direction,
    ksef_number: &KSeFNumber,
) -> Result<Invoice, XmlError> {
    let mut reader = qx::SliceReader::new(xml.as_str());
    let model = fa3::tns::Faktura::deserialize(&mut reader)
        .map_err(|e| XmlError::DeserializationFailed(format!("cannot parse FA(3) XML: {e}")))?;

    model_to_domain(model, direction, ksef_number, xml)
}

fn domain_to_model(invoice: &Invoice) -> Result<fa3::tns::FakturaElementType, XmlError> {
    let seller_nip = invoice.seller.nip.as_ref().ok_or_else(|| {
        XmlError::ValidationFailed("seller NIP is required by FA(3) schema".to_string())
    })?;

    let buyer_id = if let Some(nip) = &invoice.buyer.nip {
        vec![
            fa3::tns::TPodmiot2TypeContent::Nip(nip.as_str().to_string()),
            fa3::tns::TPodmiot2TypeContent::Nazwa(invoice.buyer.name.clone()),
        ]
    } else {
        vec![
            fa3::tns::TPodmiot2TypeContent::BrakId(
                fa3::etd::elementarne_typy_danych_v100e::TWybor1Type::_1,
            ),
            fa3::tns::TPodmiot2TypeContent::Nazwa(invoice.buyer.name.clone()),
        ]
    };

    let mut fa_content = Vec::new();
    fa_content.push(FaContent::KodWaluty(currency_to_xsd(&invoice.currency)?));
    fa_content.push(FaContent::P1(invoice.issue_date.to_string()));
    fa_content.push(FaContent::P2(invoice.invoice_number.clone()));

    if let Some(sale_date) = invoice.sale_date {
        fa_content.push(FaContent::P6(sale_date.to_string()));
    }

    for summary in vat_summary_entries(&invoice.line_items) {
        fa_content.push(summary);
    }

    fa_content.push(FaContent::P15(money_to_f64(invoice.total_gross)));
    fa_content.push(FaContent::Adnotacje(default_adnotacje()));
    fa_content.push(FaContent::RodzajFaktury(invoice_type_to_xsd(
        invoice.invoice_type,
    )?));

    for item in &invoice.line_items {
        fa_content.push(FaContent::FaWiersz(line_item_to_xsd(item)?));
    }

    if let Some(platnosc) = payment_to_xsd(invoice) {
        fa_content.push(FaContent::Platnosc(platnosc));
    }

    Ok(fa3::tns::FakturaElementType {
        naglowek: fa3::tns::TNaglowekType {
            kod_formularza: fa3::tns::TNaglowekKodFormularzaElementType {
                kod_systemowy: "FA (3)".to_string(),
                wersja_schemy: SCHEMA_VERSION.to_string(),
                content: fa3::tns::TKodFormularzaType::Fa,
            },
            wariant_formularza: fa3::tns::TNaglowekWariantFormularzaElementType::_3,
            data_wytworzenia_fa: format!("{}T00:00:00Z", invoice.issue_date),
            system_info: Some(SYSTEM_INFO.to_string()),
        },
        podmiot_1: fa3::tns::FakturaPodmiot1ElementType {
            prefiks_podatnika: None,
            nr_eori: None,
            dane_identyfikacyjne: fa3::tns::TPodmiot1Type {
                nip: seller_nip.as_str().to_string(),
                nazwa: invoice.seller.name.clone(),
            },
            adres: address_to_xsd(&invoice.seller.address)?,
            adres_koresp: None,
            dane_kontaktowe: vec![],
            status_info_podatnika: None,
        },
        podmiot_2: fa3::tns::FakturaPodmiot2ElementType {
            nr_eori: None,
            dane_identyfikacyjne: fa3::tns::TPodmiot2Type { content: buyer_id },
            adres: Some(address_to_xsd(&invoice.buyer.address)?),
            adres_koresp: None,
            dane_kontaktowe: vec![],
            nr_klienta: None,
            id_nabywcy: None,
            jst: fa3::tns::FakturaPodmiot2JstElementType::_2,
            gv: fa3::tns::FakturaPodmiot2GvElementType::_2,
        },
        podmiot_3: vec![],
        podmiot_upowazniony: None,
        fa: fa3::tns::FakturaFaElementType {
            content: fa_content,
        },
        stopka: None,
        zalacznik: None,
    })
}

fn model_to_domain(
    model: fa3::tns::FakturaElementType,
    direction: Direction,
    ksef_number: &KSeFNumber,
    raw_xml: &InvoiceXml,
) -> Result<Invoice, XmlError> {
    let seller = seller_from_xsd(&model.podmiot_1)?;
    let buyer = buyer_from_xsd(&model.podmiot_2)?;

    let mut currency = None;
    let mut issue_date = None;
    let mut invoice_number = None;
    let mut sale_date = None;
    let mut total_gross = None;
    let mut invoice_type = None;
    let mut line_items = Vec::new();
    let mut payment_method = None;
    let mut payment_deadline = None;
    let mut bank_account = None;

    for content in model.fa.content {
        match content {
            FaContent::KodWaluty(code) => {
                currency = Some(parse_currency_from_xsd(code)?);
            }
            FaContent::P1(value) => {
                issue_date = Some(parse_date_string(&value, "P_1")?);
            }
            FaContent::P2(value) => {
                invoice_number = Some(value);
            }
            FaContent::P6(value) => {
                sale_date = Some(parse_date_string(&value, "P_6")?);
            }
            FaContent::P15(value) => {
                total_gross = Some(money_from_f64("P_15", value)?);
            }
            FaContent::RodzajFaktury(kind) => {
                invoice_type = Some(invoice_type_from_xsd(kind));
            }
            FaContent::FaWiersz(row) => {
                line_items.push(line_item_from_xsd(row)?);
            }
            FaContent::Platnosc(platnosc) => {
                parse_payment(
                    &platnosc,
                    &mut payment_method,
                    &mut payment_deadline,
                    &mut bank_account,
                )?;
            }
            FaContent::Adnotacje(_)
            | FaContent::DaneFaKorygowanej(_)
            | FaContent::DodatkowyOpis(_)
            | FaContent::FakturaZaliczkowa(_)
            | FaContent::Fp(_)
            | FaContent::KursWalutyZ(_)
            | FaContent::KursWalutyZk(_)
            | FaContent::NrFaKorygowany(_)
            | FaContent::OkresFa(_)
            | FaContent::OkresFaKorygowanej(_)
            | FaContent::P131(_)
            | FaContent::P1310(_)
            | FaContent::P1311(_)
            | FaContent::P132(_)
            | FaContent::P133(_)
            | FaContent::P134(_)
            | FaContent::P135(_)
            | FaContent::P1361(_)
            | FaContent::P1362(_)
            | FaContent::P1363(_)
            | FaContent::P137(_)
            | FaContent::P138(_)
            | FaContent::P139(_)
            | FaContent::P141(_)
            | FaContent::P141W(_)
            | FaContent::P142(_)
            | FaContent::P142W(_)
            | FaContent::P143(_)
            | FaContent::P143W(_)
            | FaContent::P144(_)
            | FaContent::P144W(_)
            | FaContent::P145(_)
            | FaContent::P15Zk(_)
            | FaContent::P1M(_)
            | FaContent::Podmiot1K(_)
            | FaContent::Podmiot2K(_)
            | FaContent::PrzyczynaKorekty(_)
            | FaContent::Rozliczenie(_)
            | FaContent::Tp(_)
            | FaContent::TypKorekty(_)
            | FaContent::WarunkiTransakcji(_)
            | FaContent::Wz(_)
            | FaContent::ZaliczkaCzesciowa(_)
            | FaContent::Zamowienie(_)
            | FaContent::ZwrotAkcyzy(_) => {}
        }
    }

    let currency = currency.ok_or_else(|| XmlError::MissingElement("KodWaluty".to_string()))?;
    let issue_date = issue_date.ok_or_else(|| XmlError::MissingElement("P_1".to_string()))?;
    let invoice_number =
        invoice_number.ok_or_else(|| XmlError::MissingElement("P_2".to_string()))?;
    let total_gross = total_gross.ok_or_else(|| XmlError::MissingElement("P_15".to_string()))?;
    let invoice_type =
        invoice_type.ok_or_else(|| XmlError::MissingElement("RodzajFaktury".to_string()))?;

    let (total_net, total_vat) = compute_totals_from_items(&line_items);

    Ok(Invoice {
        id: InvoiceId::new(),
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
        raw_xml: Some(raw_xml.as_str().to_string()),
    })
}

fn address_to_xsd(address: &Address) -> Result<fa3::tns::TAdresType, XmlError> {
    Ok(fa3::tns::TAdresType {
        kod_kraju: parse_enum("KodKraju", address.country_code.as_str())?,
        adres_l1: address.line1.clone(),
        adres_l2: if address.line2.trim().is_empty() {
            None
        } else {
            Some(address.line2.clone())
        },
        gln: None,
    })
}

fn address_from_xsd(address: &fa3::tns::TAdresType) -> Result<Address, XmlError> {
    let country = enum_to_string("KodKraju", &address.kod_kraju)?;
    let country_code = CountryCode::parse(&country).map_err(|e| XmlError::InvalidValue {
        element: "KodKraju".to_string(),
        reason: e.to_string(),
    })?;

    Ok(Address {
        country_code,
        line1: address.adres_l1.clone(),
        line2: address.adres_l2.clone().unwrap_or_default(),
    })
}

fn seller_from_xsd(podmiot: &fa3::tns::FakturaPodmiot1ElementType) -> Result<Party, XmlError> {
    let nip =
        Nip::parse(&podmiot.dane_identyfikacyjne.nip).map_err(|e| XmlError::InvalidValue {
            element: "NIP".to_string(),
            reason: e.to_string(),
        })?;

    Ok(Party {
        nip: Some(nip),
        name: podmiot.dane_identyfikacyjne.nazwa.clone(),
        address: address_from_xsd(&podmiot.adres)?,
    })
}

fn buyer_from_xsd(podmiot: &fa3::tns::FakturaPodmiot2ElementType) -> Result<Party, XmlError> {
    let mut nip = None;
    let mut name = None;

    for content in &podmiot.dane_identyfikacyjne.content {
        match content {
            fa3::tns::TPodmiot2TypeContent::Nip(value) => {
                let parsed = Nip::parse(value).map_err(|e| XmlError::InvalidValue {
                    element: "NIP".to_string(),
                    reason: e.to_string(),
                })?;
                nip = Some(parsed);
            }
            fa3::tns::TPodmiot2TypeContent::Nazwa(value) => {
                name = Some(value.clone());
            }
            fa3::tns::TPodmiot2TypeContent::BrakId(_)
            | fa3::tns::TPodmiot2TypeContent::KodKraju(_)
            | fa3::tns::TPodmiot2TypeContent::KodUe(_)
            | fa3::tns::TPodmiot2TypeContent::NrId(_)
            | fa3::tns::TPodmiot2TypeContent::NrVatUe(_) => {}
        }
    }

    let name = name.ok_or_else(|| XmlError::MissingElement("Podmiot2/Nazwa".to_string()))?;
    let address = podmiot
        .adres
        .as_ref()
        .ok_or_else(|| XmlError::MissingElement("Podmiot2/Adres".to_string()))?;

    Ok(Party {
        nip,
        name,
        address: address_from_xsd(address)?,
    })
}

fn payment_to_xsd(invoice: &Invoice) -> Option<fa3::tns::FakturaFaPlatnoscElementType> {
    if invoice.payment_method.is_none()
        && invoice.payment_deadline.is_none()
        && invoice.bank_account.is_none()
    {
        return None;
    }

    let mut content = Vec::new();

    if let Some(deadline) = invoice.payment_deadline {
        content.push(
            fa3::tns::FakturaFaPlatnoscElementTypeContent::TerminPlatnosci(
                fa3::tns::FakturaFaPlatnoscTerminPlatnosciElementType {
                    content: Some(
                        fa3::tns::FakturaFaPlatnoscTerminPlatnosciElementTypeContent {
                            termin: Some(deadline.to_string()),
                            termin_opis: None,
                        },
                    ),
                },
            ),
        );
    }

    if let Some(method) = invoice.payment_method {
        content.push(
            fa3::tns::FakturaFaPlatnoscElementTypeContent::FormaPlatnosci(payment_method_to_xsd(
                method,
            )),
        );
    }

    if let Some(account) = &invoice.bank_account {
        content.push(
            fa3::tns::FakturaFaPlatnoscElementTypeContent::RachunekBankowy(
                fa3::tns::TRachunekBankowyType {
                    nr_rb: account.clone(),
                    swift: None,
                    rachunek_wlasny_banku: None,
                    nazwa_banku: None,
                    opis_rachunku: None,
                },
            ),
        );
    }

    Some(fa3::tns::FakturaFaPlatnoscElementType { content })
}

fn parse_payment(
    platnosc: &fa3::tns::FakturaFaPlatnoscElementType,
    payment_method: &mut Option<PaymentMethod>,
    payment_deadline: &mut Option<NaiveDate>,
    bank_account: &mut Option<String>,
) -> Result<(), XmlError> {
    for content in &platnosc.content {
        match content {
            fa3::tns::FakturaFaPlatnoscElementTypeContent::FormaPlatnosci(value) => {
                *payment_method = Some(payment_method_from_xsd(value));
            }
            fa3::tns::FakturaFaPlatnoscElementTypeContent::TerminPlatnosci(value) => {
                if let Some(term) = &value.content {
                    if let Some(raw) = &term.termin {
                        *payment_deadline = Some(parse_date_string(raw, "Termin")?);
                    }
                }
            }
            fa3::tns::FakturaFaPlatnoscElementTypeContent::RachunekBankowy(value) => {
                *bank_account = Some(value.nr_rb.clone());
            }
            fa3::tns::FakturaFaPlatnoscElementTypeContent::DataZaplaty(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::IpkSeF(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::LinkDoPlatnosci(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::OpisPlatnosci(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::PlatnoscInna(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::RachunekBankowyFaktora(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::Skonto(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::Zaplacono(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::ZaplataCzesciowa(_)
            | fa3::tns::FakturaFaPlatnoscElementTypeContent::ZnacznikZaplatyCzesciowej(_) => {}
        }
    }

    Ok(())
}

fn line_item_to_xsd(item: &LineItem) -> Result<fa3::tns::FakturaFaFaWierszElementType, XmlError> {
    let line_number = usize::try_from(item.line_number).map_err(|_| XmlError::InvalidValue {
        element: "NrWierszaFa".to_string(),
        reason: "line number does not fit usize".to_string(),
    })?;

    let quantity =
        item.quantity
            .to_string()
            .parse::<f64>()
            .map_err(|e| XmlError::InvalidValue {
                element: "P_8B".to_string(),
                reason: format!("invalid quantity: {e}"),
            })?;

    Ok(fa3::tns::FakturaFaFaWierszElementType {
        nr_wiersza_fa: line_number,
        uu_id: None,
        p6a: None,
        p7: Some(item.description.clone()),
        indeks: None,
        gtin: None,
        pk_wi_u: None,
        cn: None,
        pkob: None,
        p8a: item.unit.clone(),
        p8b: Some(quantity),
        p9a: item.unit_net_price.map(money_to_f64),
        p9b: None,
        p10: None,
        p11: Some(money_to_f64(item.net_value)),
        p11a: None,
        p11_vat: None,
        p12: Some(vat_rate_to_xsd(item.vat_rate)),
        p12_xii: None,
        p12_zal_15: None,
        kwota_akcyzy: None,
        gtu: None,
        procedura: None,
        kurs_waluty: None,
        stan_przed: None,
    })
}

fn line_item_from_xsd(item: fa3::tns::FakturaFaFaWierszElementType) -> Result<LineItem, XmlError> {
    let line_number = u32::try_from(item.nr_wiersza_fa).map_err(|_| XmlError::InvalidValue {
        element: "NrWierszaFa".to_string(),
        reason: "line number does not fit u32".to_string(),
    })?;

    let description = item
        .p7
        .ok_or_else(|| XmlError::MissingElement("P_7".to_string()))?;

    let quantity_raw = item
        .p8b
        .ok_or_else(|| XmlError::MissingElement("P_8B".to_string()))?;
    let quantity = parse_quantity_from_f64(quantity_raw)?;

    let unit_net_price = item
        .p9a
        .map(|value| money_from_f64("P_9A", value))
        .transpose()?;

    let net_value_raw = item
        .p11
        .or(item.p11a)
        .ok_or_else(|| XmlError::MissingElement("P_11".to_string()))?;
    let net_value = money_from_f64("P_11", net_value_raw)?;

    let vat_rate = item
        .p12
        .ok_or_else(|| XmlError::MissingElement("P_12".to_string()))
        .map(vat_rate_from_xsd)?;

    let vat_grosze = match vat_rate.percentage() {
        Some(percent) => {
            let numerator = i128::from(net_value.grosze()) * i128::from(percent);
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
        unit: item.p8a,
        quantity,
        unit_net_price,
        net_value,
        vat_rate,
        vat_amount,
        gross_value,
    })
}

fn vat_summary_entries(items: &[LineItem]) -> Vec<FaContent> {
    let mut totals: BTreeMap<&'static str, (Money, Money)> = BTreeMap::new();

    for item in items {
        let key = match item.vat_rate {
            VatRate::Rate22 | VatRate::Rate23 => "r23",
            VatRate::Rate8 | VatRate::Rate7 => "r8",
            VatRate::Rate5 => "r5",
            VatRate::Rate4 => "r4",
            VatRate::Rate3 => "r3",
            VatRate::Rate0 => "r0",
            VatRate::Exempt => "zw",
            VatRate::NotSubject => "oo",
            VatRate::ReverseCharge => "np",
        };

        let entry = totals
            .entry(key)
            .or_insert((Money::from_grosze(0), Money::from_grosze(0)));
        entry.0 = entry.0 + item.net_value;
        entry.1 = entry.1 + item.vat_amount;
    }

    let mut out = Vec::new();

    if let Some((net, vat)) = totals.get("r23") {
        out.push(FaContent::P131(money_to_f64(*net)));
        out.push(FaContent::P141(money_to_f64(*vat)));
    }
    if let Some((net, vat)) = totals.get("r8") {
        out.push(FaContent::P132(money_to_f64(*net)));
        out.push(FaContent::P142(money_to_f64(*vat)));
    }
    if let Some((net, vat)) = totals.get("r5") {
        out.push(FaContent::P133(money_to_f64(*net)));
        out.push(FaContent::P143(money_to_f64(*vat)));
    }
    if let Some((net, vat)) = totals.get("r4") {
        out.push(FaContent::P134(money_to_f64(*net)));
        out.push(FaContent::P144(money_to_f64(*vat)));
    }
    if let Some((net, vat)) = totals.get("r3") {
        out.push(FaContent::P135(money_to_f64(*net)));
        out.push(FaContent::P145(money_to_f64(*vat)));
    }
    if let Some((net, _)) = totals.get("r0") {
        out.push(FaContent::P1361(money_to_f64(*net)));
    }
    if let Some((net, _)) = totals.get("zw") {
        out.push(FaContent::P137(money_to_f64(*net)));
    }
    if let Some((net, _)) = totals.get("oo") {
        out.push(FaContent::P138(money_to_f64(*net)));
    }
    if let Some((net, _)) = totals.get("np") {
        out.push(FaContent::P139(money_to_f64(*net)));
    }

    out
}

fn default_adnotacje() -> fa3::tns::FakturaFaAdnotacjeElementType {
    fa3::tns::FakturaFaAdnotacjeElementType {
        p16: fa3::etd::elementarne_typy_danych_v100e::TWybor12Type::_2,
        p17: fa3::etd::elementarne_typy_danych_v100e::TWybor12Type::_2,
        p18: fa3::etd::elementarne_typy_danych_v100e::TWybor12Type::_2,
        p18a: fa3::etd::elementarne_typy_danych_v100e::TWybor12Type::_2,
        zwolnienie: fa3::tns::FakturaFaAdnotacjeZwolnienieElementType {
            content: vec![
                fa3::tns::FakturaFaAdnotacjeZwolnienieElementTypeContent::P19N(
                    fa3::etd::elementarne_typy_danych_v100e::TWybor1Type::_1,
                ),
            ],
        },
        nowe_srodki_transportu: fa3::tns::FakturaFaAdnotacjeNoweSrodkiTransportuElementType {
            content: vec![
                fa3::tns::FakturaFaAdnotacjeNoweSrodkiTransportuElementTypeContent::P22N(
                    fa3::etd::elementarne_typy_danych_v100e::TWybor1Type::_1,
                ),
            ],
        },
        p23: fa3::etd::elementarne_typy_danych_v100e::TWybor12Type::_2,
        p_marzy: fa3::tns::FakturaFaAdnotacjePMarzyElementType {
            content: vec![
                fa3::tns::FakturaFaAdnotacjePMarzyElementTypeContent::PPMarzyN(
                    fa3::etd::elementarne_typy_danych_v100e::TWybor1Type::_1,
                ),
            ],
        },
    }
}

fn invoice_type_to_xsd(
    invoice_type: InvoiceType,
) -> Result<fa3::tns::TRodzajFakturyType, XmlError> {
    match invoice_type {
        InvoiceType::Vat => Ok(fa3::tns::TRodzajFakturyType::Vat),
        InvoiceType::Kor => Ok(fa3::tns::TRodzajFakturyType::Kor),
        InvoiceType::Zal => Ok(fa3::tns::TRodzajFakturyType::Zal),
        InvoiceType::Roz => Ok(fa3::tns::TRodzajFakturyType::Roz),
        InvoiceType::Upr => Ok(fa3::tns::TRodzajFakturyType::Upr),
        InvoiceType::KorZal => Ok(fa3::tns::TRodzajFakturyType::KorZal),
        InvoiceType::KorRoz => Ok(fa3::tns::TRodzajFakturyType::KorRoz),
        unsupported => Err(XmlError::ValidationFailed(format!(
            "unsupported invoice type for FA(3) serializer: {unsupported}"
        ))),
    }
}

fn invoice_type_from_xsd(invoice_type: fa3::tns::TRodzajFakturyType) -> InvoiceType {
    match invoice_type {
        fa3::tns::TRodzajFakturyType::Vat => InvoiceType::Vat,
        fa3::tns::TRodzajFakturyType::Kor => InvoiceType::Kor,
        fa3::tns::TRodzajFakturyType::Zal => InvoiceType::Zal,
        fa3::tns::TRodzajFakturyType::Roz => InvoiceType::Roz,
        fa3::tns::TRodzajFakturyType::Upr => InvoiceType::Upr,
        fa3::tns::TRodzajFakturyType::KorZal => InvoiceType::KorZal,
        fa3::tns::TRodzajFakturyType::KorRoz => InvoiceType::KorRoz,
    }
}

fn payment_method_to_xsd(method: PaymentMethod) -> fa3::tns::TFormaPlatnosciType {
    match method {
        PaymentMethod::Cash => fa3::tns::TFormaPlatnosciType::_1,
        PaymentMethod::Card => fa3::tns::TFormaPlatnosciType::_2,
        PaymentMethod::Voucher => fa3::tns::TFormaPlatnosciType::_3,
        PaymentMethod::Check => fa3::tns::TFormaPlatnosciType::_4,
        PaymentMethod::Credit => fa3::tns::TFormaPlatnosciType::_5,
        PaymentMethod::Transfer => fa3::tns::TFormaPlatnosciType::_6,
        PaymentMethod::Mobile => fa3::tns::TFormaPlatnosciType::_7,
    }
}

fn payment_method_from_xsd(method: &fa3::tns::TFormaPlatnosciType) -> PaymentMethod {
    match method {
        fa3::tns::TFormaPlatnosciType::_1 => PaymentMethod::Cash,
        fa3::tns::TFormaPlatnosciType::_2 => PaymentMethod::Card,
        fa3::tns::TFormaPlatnosciType::_3 => PaymentMethod::Voucher,
        fa3::tns::TFormaPlatnosciType::_4 => PaymentMethod::Check,
        fa3::tns::TFormaPlatnosciType::_5 => PaymentMethod::Credit,
        fa3::tns::TFormaPlatnosciType::_6 => PaymentMethod::Transfer,
        fa3::tns::TFormaPlatnosciType::_7 => PaymentMethod::Mobile,
    }
}

fn vat_rate_to_xsd(vat_rate: VatRate) -> fa3::tns::TStawkaPodatkuType {
    match vat_rate {
        VatRate::Rate23 => fa3::tns::TStawkaPodatkuType::_23,
        VatRate::Rate22 => fa3::tns::TStawkaPodatkuType::_22,
        VatRate::Rate8 => fa3::tns::TStawkaPodatkuType::_8,
        VatRate::Rate7 => fa3::tns::TStawkaPodatkuType::_7,
        VatRate::Rate5 => fa3::tns::TStawkaPodatkuType::_5,
        VatRate::Rate4 => fa3::tns::TStawkaPodatkuType::_4,
        VatRate::Rate3 => fa3::tns::TStawkaPodatkuType::_3,
        VatRate::Rate0 => fa3::tns::TStawkaPodatkuType::_0Kr,
        VatRate::Exempt => fa3::tns::TStawkaPodatkuType::Zw,
        VatRate::NotSubject => fa3::tns::TStawkaPodatkuType::Oo,
        VatRate::ReverseCharge => fa3::tns::TStawkaPodatkuType::NpI,
    }
}

fn vat_rate_from_xsd(vat_rate: fa3::tns::TStawkaPodatkuType) -> VatRate {
    match vat_rate {
        fa3::tns::TStawkaPodatkuType::_23 => VatRate::Rate23,
        fa3::tns::TStawkaPodatkuType::_22 => VatRate::Rate22,
        fa3::tns::TStawkaPodatkuType::_8 => VatRate::Rate8,
        fa3::tns::TStawkaPodatkuType::_7 => VatRate::Rate7,
        fa3::tns::TStawkaPodatkuType::_5 => VatRate::Rate5,
        fa3::tns::TStawkaPodatkuType::_4 => VatRate::Rate4,
        fa3::tns::TStawkaPodatkuType::_3 => VatRate::Rate3,
        fa3::tns::TStawkaPodatkuType::_0Kr
        | fa3::tns::TStawkaPodatkuType::_0Wdt
        | fa3::tns::TStawkaPodatkuType::_0Ex => VatRate::Rate0,
        fa3::tns::TStawkaPodatkuType::Zw => VatRate::Exempt,
        fa3::tns::TStawkaPodatkuType::Oo => VatRate::NotSubject,
        fa3::tns::TStawkaPodatkuType::NpI | fa3::tns::TStawkaPodatkuType::NpIi => {
            VatRate::ReverseCharge
        }
    }
}

fn parse_currency_from_xsd(code: fa3::tns::TKodWalutyType) -> Result<Currency, XmlError> {
    let code = enum_to_string("KodWaluty", &code)?;
    Currency::parse(&code).map_err(|e| XmlError::InvalidValue {
        element: "KodWaluty".to_string(),
        reason: e.to_string(),
    })
}

fn currency_to_xsd(currency: &Currency) -> Result<fa3::tns::TKodWalutyType, XmlError> {
    parse_enum("KodWaluty", currency.as_str())
}

fn parse_enum<T>(element: &str, value: &str) -> Result<T, XmlError>
where
    T: DeserializeBytes,
{
    let mut helper = qx::DeserializeHelper::default();
    T::deserialize_bytes(&mut helper, value.as_bytes()).map_err(|e| XmlError::InvalidValue {
        element: element.to_string(),
        reason: format!("'{value}' is not valid for {element}: {e}"),
    })
}

fn enum_to_string<T>(element: &str, value: &T) -> Result<String, XmlError>
where
    T: SerializeBytes,
{
    let mut helper = qx::SerializeHelper::default();
    value
        .serialize_bytes(&mut helper)
        .map_err(|e| XmlError::InvalidValue {
            element: element.to_string(),
            reason: format!("cannot serialize enum value: {e}"),
        })?
        .map(|cow| cow.into_owned())
        .ok_or_else(|| XmlError::InvalidValue {
            element: element.to_string(),
            reason: "enum value produced empty serialization".to_string(),
        })
}

fn parse_date_string(raw: &str, element: &str) -> Result<NaiveDate, XmlError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .or_else(|_| {
            raw.get(0..10)
                .and_then(|prefix| NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok())
                .ok_or(())
        })
        .map_err(|()| XmlError::InvalidValue {
            element: element.to_string(),
            reason: format!("cannot parse date: '{raw}'"),
        })
}

fn money_to_f64(money: Money) -> f64 {
    money.grosze() as f64 / 100.0
}

fn money_from_f64(element: &str, value: f64) -> Result<Money, XmlError> {
    if !value.is_finite() {
        return Err(XmlError::InvalidValue {
            element: element.to_string(),
            reason: "value is not finite".to_string(),
        });
    }

    let rounded = (value * 100.0).round();
    if rounded < i64::MIN as f64 || rounded > i64::MAX as f64 {
        return Err(XmlError::InvalidValue {
            element: element.to_string(),
            reason: "money value overflows i64".to_string(),
        });
    }

    Ok(Money::from_grosze(rounded as i64))
}

fn parse_quantity_from_f64(value: f64) -> Result<Quantity, XmlError> {
    if !value.is_finite() {
        return Err(XmlError::InvalidValue {
            element: "P_8B".to_string(),
            reason: "quantity is not finite".to_string(),
        });
    }

    let mut rendered = format!("{value:.6}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    if rendered == "-0" {
        rendered = "0".to_string();
    }

    Quantity::parse(&rendered).map_err(|e| XmlError::InvalidValue {
        element: "P_8B".to_string(),
        reason: e.to_string(),
    })
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
    use crate::infra::fa3::Fa3XsdValidator;
    use crate::ports::invoice_xml_validator::InvoiceXmlValidator;
    use crate::test_support::fixtures::sample_invoice;

    #[test]
    fn generated_model_round_trip() {
        let original = sample_invoice();
        let xml = invoice_to_xml(&original).expect("serialize to XML");
        let parsed = xml_to_invoice(
            &xml,
            Direction::Outgoing,
            &KSeFNumber::new("KSeF-TEST-001".to_string()),
        )
        .expect("parse XML");

        assert_eq!(parsed.invoice_type, original.invoice_type);
        assert_eq!(parsed.invoice_number, original.invoice_number);
        assert_eq!(parsed.issue_date, original.issue_date);
        assert_eq!(parsed.sale_date, original.sale_date);
        assert_eq!(parsed.seller.nip, original.seller.nip);
        assert_eq!(parsed.buyer.nip, original.buyer.nip);
        assert_eq!(parsed.currency.as_str(), original.currency.as_str());
        assert_eq!(parsed.total_gross, original.total_gross);
        assert_eq!(parsed.total_net, original.total_net);
        assert_eq!(parsed.total_vat, original.total_vat);
        assert_eq!(parsed.line_items.len(), original.line_items.len());
        assert_eq!(parsed.payment_method, original.payment_method);
        assert_eq!(parsed.payment_deadline, original.payment_deadline);
        assert_eq!(parsed.bank_account, original.bank_account);
    }

    #[test]
    fn generated_xml_passes_pinned_xsd_validation() {
        let invoice = sample_invoice();
        let xml = invoice_to_xml(&invoice).expect("serialize to XML");

        let validator = Fa3XsdValidator::new();
        validator
            .validate(&xml)
            .expect("generated XML should pass pinned FA(3) XSD");
    }

    #[test]
    fn schema_change_sentinel_enums_are_exhaustively_handled() {
        fn fa_content_sentinel(value: FaContent) {
            match value {
                FaContent::Adnotacje(_)
                | FaContent::DaneFaKorygowanej(_)
                | FaContent::DodatkowyOpis(_)
                | FaContent::FaWiersz(_)
                | FaContent::FakturaZaliczkowa(_)
                | FaContent::Fp(_)
                | FaContent::KodWaluty(_)
                | FaContent::KursWalutyZ(_)
                | FaContent::KursWalutyZk(_)
                | FaContent::NrFaKorygowany(_)
                | FaContent::OkresFa(_)
                | FaContent::OkresFaKorygowanej(_)
                | FaContent::P1(_)
                | FaContent::P131(_)
                | FaContent::P1310(_)
                | FaContent::P1311(_)
                | FaContent::P132(_)
                | FaContent::P133(_)
                | FaContent::P134(_)
                | FaContent::P135(_)
                | FaContent::P1361(_)
                | FaContent::P1362(_)
                | FaContent::P1363(_)
                | FaContent::P137(_)
                | FaContent::P138(_)
                | FaContent::P139(_)
                | FaContent::P141(_)
                | FaContent::P141W(_)
                | FaContent::P142(_)
                | FaContent::P142W(_)
                | FaContent::P143(_)
                | FaContent::P143W(_)
                | FaContent::P144(_)
                | FaContent::P144W(_)
                | FaContent::P145(_)
                | FaContent::P15(_)
                | FaContent::P15Zk(_)
                | FaContent::P1M(_)
                | FaContent::P2(_)
                | FaContent::P6(_)
                | FaContent::Platnosc(_)
                | FaContent::Podmiot1K(_)
                | FaContent::Podmiot2K(_)
                | FaContent::PrzyczynaKorekty(_)
                | FaContent::RodzajFaktury(_)
                | FaContent::Rozliczenie(_)
                | FaContent::Tp(_)
                | FaContent::TypKorekty(_)
                | FaContent::WarunkiTransakcji(_)
                | FaContent::Wz(_)
                | FaContent::ZaliczkaCzesciowa(_)
                | FaContent::Zamowienie(_)
                | FaContent::ZwrotAkcyzy(_) => {}
            }
        }

        fn podmiot2_sentinel(value: fa3::tns::TPodmiot2TypeContent) {
            match value {
                fa3::tns::TPodmiot2TypeContent::BrakId(_)
                | fa3::tns::TPodmiot2TypeContent::KodKraju(_)
                | fa3::tns::TPodmiot2TypeContent::KodUe(_)
                | fa3::tns::TPodmiot2TypeContent::Nazwa(_)
                | fa3::tns::TPodmiot2TypeContent::Nip(_)
                | fa3::tns::TPodmiot2TypeContent::NrId(_)
                | fa3::tns::TPodmiot2TypeContent::NrVatUe(_) => {}
            }
        }

        fn platnosc_sentinel(value: fa3::tns::FakturaFaPlatnoscElementTypeContent) {
            match value {
                fa3::tns::FakturaFaPlatnoscElementTypeContent::DataZaplaty(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::FormaPlatnosci(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::IpkSeF(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::LinkDoPlatnosci(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::OpisPlatnosci(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::PlatnoscInna(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::RachunekBankowy(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::RachunekBankowyFaktora(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::Skonto(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::TerminPlatnosci(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::Zaplacono(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::ZaplataCzesciowa(_)
                | fa3::tns::FakturaFaPlatnoscElementTypeContent::ZnacznikZaplatyCzesciowej(_) => {}
            }
        }

        let _fa: fn(FaContent) = fa_content_sentinel;
        let _podmiot2: fn(fa3::tns::TPodmiot2TypeContent) = podmiot2_sentinel;
        let _platnosc: fn(fa3::tns::FakturaFaPlatnoscElementTypeContent) = platnosc_sentinel;
    }
}
