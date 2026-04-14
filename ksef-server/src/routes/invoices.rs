use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::NaiveDate;
use serde::Deserialize;

use ksef_core::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceType, LineItem, Money,
    Party, PaymentMethod, Quantity, VatRate,
};
use ksef_core::domain::nip::Nip;
use ksef_core::error::RepositoryError;
use ksef_core::ports::invoice_repository::InvoiceFilter;
use ksef_core::services::invoice_service::{CreateInvoiceInput, InvoiceServiceError};

use crate::state::AppState;

// --- Templates ---

#[derive(Template)]
#[template(path = "pages/invoices.html")]
struct InvoiceListTemplate {
    active: &'static str,
    outgoing: Vec<Invoice>,
    incoming: Vec<Invoice>,
    tab: String,
}

#[derive(Template)]
#[template(path = "pages/invoice_detail.html")]
struct InvoiceDetailTemplate {
    active: &'static str,
    invoice: Invoice,
}

#[derive(Template)]
#[template(path = "pages/invoice_new.html")]
struct InvoiceNewTemplate {
    active: &'static str,
    error: Option<String>,
    default_nip: String,
    today: String,
    payment_deadline: String,
}

fn render<T: Template>(tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {e}"),
        )
            .into_response(),
    }
}

fn render_with_status<T: Template>(status: StatusCode, tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {e}"),
        )
            .into_response(),
    }
}

fn status_for_service_error(err: &InvoiceServiceError) -> StatusCode {
    match err {
        InvoiceServiceError::Repository(RepositoryError::NotFound { .. }) => StatusCode::NOT_FOUND,
        InvoiceServiceError::Domain(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// --- Form data ---

#[derive(Deserialize)]
pub struct InvoiceFormData {
    pub invoice_number: String,
    pub issue_date: String,
    pub sale_date: String,
    pub seller_nip: String,
    pub seller_name: String,
    pub seller_address_line1: String,
    pub seller_address_line2: String,
    pub buyer_nip: String,
    pub buyer_name: String,
    pub buyer_address_line1: String,
    pub buyer_address_line2: String,
    pub item_description: String,
    pub item_quantity: String,
    pub item_unit_price: String,
    pub item_vat_rate: String,
    pub payment_method: String,
    pub payment_deadline: String,
    pub bank_account: Option<String>,
}

// --- Handlers ---

pub async fn list(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let invoices = match state.invoice_service.list(&InvoiceFilter::default()).await {
        Ok(invoices) => invoices,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udalo sie pobrac listy faktur: {err}"),
            )
                .into_response();
        }
    };

    let tab = params
        .get("tab")
        .cloned()
        .unwrap_or_else(|| "outgoing".to_string());

    let (outgoing, incoming): (Vec<_>, Vec<_>) = invoices
        .into_iter()
        .partition(|inv| inv.direction == Direction::Outgoing);

    render(InvoiceListTemplate {
        active: "/invoices",
        outgoing,
        incoming,
        tab,
    })
}

fn new_form_defaults(state: &AppState) -> InvoiceNewTemplate {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let deadline = (chrono::Local::now() + chrono::Duration::days(14))
        .format("%Y-%m-%d")
        .to_string();
    InvoiceNewTemplate {
        active: "/invoices",
        error: None,
        default_nip: state.nip.as_str().to_string(),
        today,
        payment_deadline: deadline,
    }
}

pub async fn new_form(State(state): State<AppState>) -> impl IntoResponse {
    render(new_form_defaults(&state))
}

pub async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let invoice_id: InvoiceId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Nieprawidlowy identyfikator faktury: {id}"),
            )
                .into_response();
        }
    };

    match state.invoice_service.find(&invoice_id).await {
        Ok(invoice) => render(InvoiceDetailTemplate {
            active: "/invoices",
            invoice,
        })
        .into_response(),
        Err(err) => (
            status_for_service_error(&err),
            format!("Nie udalo sie pobrac faktury {invoice_id}: {err}"),
        )
            .into_response(),
    }
}

pub async fn create(State(state): State<AppState>, Form(form): Form<InvoiceFormData>) -> Response {
    match parse_form_to_input(form) {
        Ok(input) => match state.invoice_service.create_draft(input).await {
            Ok(invoice) => Redirect::to(&format!("/invoices/{}", invoice.id)).into_response(),
            Err(e) => {
                let mut tmpl = new_form_defaults(&state);
                tmpl.error = Some(format!("Nie udalo sie utworzyc faktury: {e}"));
                render_with_status(status_for_service_error(&e), tmpl)
            }
        },
        Err(e) => {
            let mut tmpl = new_form_defaults(&state);
            tmpl.error = Some(e);
            render_with_status(StatusCode::BAD_REQUEST, tmpl)
        }
    }
}

pub async fn submit(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let invoice_id: InvoiceId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Nieprawidlowy identyfikator faktury: {id}"),
            )
                .into_response();
        }
    };

    match state.invoice_service.submit(&invoice_id).await {
        Ok(()) => Redirect::to(&format!("/invoices/{invoice_id}")).into_response(),
        Err(err) => (
            status_for_service_error(&err),
            format!("Nie udalo sie wyslac faktury {invoice_id} do kolejki: {err}"),
        )
            .into_response(),
    }
}

// --- Form parsing ---

fn parse_form_to_input(form: InvoiceFormData) -> Result<CreateInvoiceInput, String> {
    let seller_nip = Nip::parse(&form.seller_nip).map_err(|e| format!("NIP sprzedawcy: {e}"))?;
    let buyer_nip = Nip::parse(&form.buyer_nip).map_err(|e| format!("NIP nabywcy: {e}"))?;
    let issue_date = NaiveDate::parse_from_str(&form.issue_date, "%Y-%m-%d")
        .map_err(|e| format!("Data wystawienia: {e}"))?;
    let sale_date = NaiveDate::parse_from_str(&form.sale_date, "%Y-%m-%d")
        .map_err(|e| format!("Data sprzedazy: {e}"))?;
    let payment_deadline = NaiveDate::parse_from_str(&form.payment_deadline, "%Y-%m-%d")
        .map_err(|e| format!("Termin platnosci: {e}"))?;

    let quantity = Quantity::parse(&form.item_quantity).map_err(|e| format!("Ilosc: {e}"))?;
    let unit_price: Money = form
        .item_unit_price
        .parse()
        .map_err(|e| format!("Cena jednostkowa: {e}"))?;
    let vat_rate: VatRate = form
        .item_vat_rate
        .parse()
        .map_err(|e| format!("Stawka VAT: {e}"))?;

    let net = multiply_money_by_quantity(unit_price, &quantity)
        .map_err(|e| format!("Wartosc netto: {e}"))?;
    let net_grosze = net.grosze();

    let vat_grosze = match vat_rate.percentage() {
        Some(pct) => {
            let vat_numerator = i128::from(net_grosze) * i128::from(pct);
            let rounded = div_round_half_away_from_zero(vat_numerator, 100);
            i64::try_from(rounded)
                .map_err(|_| "wartosc VAT przekracza zakres obslugiwanych kwot".to_string())?
        }
        None => 0,
    };
    let vat = Money::from_grosze(vat_grosze);
    let gross = Money::from_grosze(net_grosze + vat_grosze);

    let payment_method = match form.payment_method.as_str() {
        "cash" => PaymentMethod::Cash,
        "card" => PaymentMethod::Card,
        "transfer" => PaymentMethod::Transfer,
        other => {
            return Err(format!(
                "Metoda platnosci: nieobslugiwana wartosc '{other}' (oczekiwane: transfer, cash, card)"
            ));
        }
    };

    let line_item = LineItem {
        line_number: 1,
        description: form.item_description,
        unit: None,
        quantity,
        unit_net_price: Some(unit_price),
        net_value: net,
        vat_rate,
        vat_amount: vat,
        gross_value: gross,
    };

    Ok(CreateInvoiceInput {
        direction: Direction::Outgoing,
        invoice_type: InvoiceType::Vat,
        invoice_number: form.invoice_number,
        issue_date,
        sale_date,
        corrected_invoice_number: None,
        correction_reason: None,
        original_ksef_number: None,
        advance_payment_date: None,
        seller: Party {
            nip: Some(seller_nip),
            name: form.seller_name,
            address: Address {
                country_code: CountryCode::pl(),
                line1: form.seller_address_line1,
                line2: form.seller_address_line2,
            },
        },
        buyer: Party {
            nip: Some(buyer_nip),
            name: form.buyer_name,
            address: Address {
                country_code: CountryCode::pl(),
                line1: form.buyer_address_line1,
                line2: form.buyer_address_line2,
            },
        },
        currency: Currency::pln(),
        line_items: vec![line_item],
        payment_method,
        payment_deadline,
        bank_account: form.bank_account.filter(|s| !s.is_empty()),
    })
}

fn multiply_money_by_quantity(unit_price: Money, quantity: &Quantity) -> Result<Money, String> {
    let denominator = 10_i128.pow(u32::from(quantity.scale()));
    let numerator = i128::from(unit_price.grosze()) * i128::from(quantity.value());
    let rounded = div_round_half_away_from_zero(numerator, denominator);
    let grosze = i64::try_from(rounded)
        .map_err(|_| "wartosc przekracza zakres obslugiwanych kwot".to_string())?;
    Ok(Money::from_grosze(grosze))
}

fn div_round_half_away_from_zero(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder == 0 {
        return quotient;
    }

    let should_round = remainder.abs() * 2 >= denominator.abs();
    if should_round {
        quotient + numerator.signum()
    } else {
        quotient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_form() -> InvoiceFormData {
        InvoiceFormData {
            invoice_number: "FV/2026/04/001".to_string(),
            issue_date: "2026-04-13".to_string(),
            sale_date: "2026-04-13".to_string(),
            seller_nip: "5260250274".to_string(),
            seller_name: "Seller".to_string(),
            seller_address_line1: "ul. A 1".to_string(),
            seller_address_line2: "00-001 Warszawa".to_string(),
            buyer_nip: "5260250274".to_string(),
            buyer_name: "Buyer".to_string(),
            buyer_address_line1: "ul. B 2".to_string(),
            buyer_address_line2: "00-002 Krakow".to_string(),
            item_description: "Usluga".to_string(),
            item_quantity: "1".to_string(),
            item_unit_price: "100.00".to_string(),
            item_vat_rate: "23".to_string(),
            payment_method: "transfer".to_string(),
            payment_deadline: "2026-04-27".to_string(),
            bank_account: Some("PL61109010140000071219812874".to_string()),
        }
    }

    #[test]
    fn parse_form_supports_decimal_quantity() {
        let mut form = base_form();
        form.item_quantity = "1.5".to_string();

        let input = parse_form_to_input(form).unwrap();
        let item = &input.line_items[0];
        assert_eq!(item.net_value.grosze(), 15_000);
        assert_eq!(item.vat_amount.grosze(), 3_450);
        assert_eq!(item.gross_value.grosze(), 18_450);
    }

    #[test]
    fn parse_form_rounds_vat_to_full_grosz() {
        let mut form = base_form();
        form.item_quantity = "0.03".to_string();
        form.item_unit_price = "1.00".to_string();

        let input = parse_form_to_input(form).unwrap();
        let item = &input.line_items[0];
        assert_eq!(item.net_value.grosze(), 3);
        assert_eq!(item.vat_amount.grosze(), 1);
        assert_eq!(item.gross_value.grosze(), 4);
    }

    #[test]
    fn parse_form_invalid_seller_nip_fails() {
        let mut form = base_form();
        form.seller_nip = "123".to_string();
        let err = parse_form_to_input(form).unwrap_err();
        assert!(err.contains("NIP sprzedawcy"));
    }

    #[test]
    fn parse_form_invalid_buyer_nip_fails() {
        let mut form = base_form();
        form.buyer_nip = "abc".to_string();
        let err = parse_form_to_input(form).unwrap_err();
        assert!(err.contains("NIP nabywcy"));
    }

    #[test]
    fn parse_form_invalid_issue_date_fails() {
        let mut form = base_form();
        form.issue_date = "not-a-date".to_string();
        let err = parse_form_to_input(form).unwrap_err();
        assert!(err.contains("Data wystawienia"));
    }

    #[test]
    fn parse_form_invalid_payment_method_fails() {
        let mut form = base_form();
        form.payment_method = "bitcoin".to_string();
        let err = parse_form_to_input(form).unwrap_err();
        assert!(err.contains("Metoda platnosci"));
    }

    #[test]
    fn parse_form_empty_bank_account_is_none() {
        let mut form = base_form();
        form.bank_account = Some(String::new());
        let input = parse_form_to_input(form).unwrap();
        assert!(input.bank_account.is_none());
    }

    #[test]
    fn parse_form_exempt_vat_rate() {
        let mut form = base_form();
        form.item_vat_rate = "zw".to_string();
        let input = parse_form_to_input(form).unwrap();
        let item = &input.line_items[0];
        assert_eq!(item.vat_amount.grosze(), 0);
        assert_eq!(item.gross_value.grosze(), item.net_value.grosze());
    }

    #[test]
    fn parse_form_zero_quantity_fails() {
        let mut form = base_form();
        form.item_quantity = "abc".to_string();
        let err = parse_form_to_input(form).unwrap_err();
        assert!(err.contains("Ilosc"));
    }

    #[test]
    fn parse_form_default_values_are_correct() {
        let form = base_form();
        let input = parse_form_to_input(form).unwrap();
        assert_eq!(input.direction, Direction::Outgoing);
        assert_eq!(input.invoice_type, InvoiceType::Vat);
        assert!(input.corrected_invoice_number.is_none());
        assert!(input.correction_reason.is_none());
    }
}
