use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use tower_sessions::Session;

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceType, LineItem, Money,
    Party, PaymentMethod, Quantity, VatRate, format_invoice_number,
};
use ksef_core::domain::nip::Nip;
use ksef_core::error::RepositoryError;
use ksef_core::ports::invoice_repository::InvoiceFilter;
use ksef_core::services::invoice_service::{CreateInvoiceInput, InvoiceServiceError};

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::{CsrfForm, NipContext};
use crate::request_meta::client_ip;
use crate::state::AppState;

// --- Templates ---

#[derive(Template)]
#[template(path = "pages/invoices.html")]
struct InvoiceListTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    outgoing: Vec<Invoice>,
    incoming: Vec<Invoice>,
    tab: String,
    fetch_started: bool,
}

#[derive(Template)]
#[template(path = "pages/invoice_detail.html")]
struct InvoiceDetailTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    invoice: Invoice,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "pages/invoice_new.html")]
struct InvoiceNewTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    error: Option<String>,
    f: InvoiceFormValues,
    csrf_token: String,
}

/// Values to pre-fill in the invoice form. Separate from `InvoiceFormData`
/// to allow defaults on fresh load.
struct InvoiceFormValues {
    invoice_number: String,
    issue_date: String,
    sale_date: String,
    seller_nip: String,
    seller_name: String,
    seller_address_line1: String,
    seller_address_line2: String,
    buyer_nip: String,
    buyer_name: String,
    buyer_address_line1: String,
    buyer_address_line2: String,
    item_description: String,
    item_quantity: String,
    item_unit_price: String,
    item_vat_rate: String,
    payment_method: String,
    payment_deadline: String,
    bank_account: String,
}

impl From<InvoiceFormData> for InvoiceFormValues {
    fn from(fd: InvoiceFormData) -> Self {
        Self {
            invoice_number: fd.invoice_number,
            issue_date: fd.issue_date,
            sale_date: fd.sale_date,
            seller_nip: fd.seller_nip,
            seller_name: fd.seller_name,
            seller_address_line1: fd.seller_address_line1,
            seller_address_line2: fd.seller_address_line2,
            buyer_nip: fd.buyer_nip,
            buyer_name: fd.buyer_name,
            buyer_address_line1: fd.buyer_address_line1,
            buyer_address_line2: fd.buyer_address_line2,
            item_description: fd.item_description,
            item_quantity: fd.item_quantity,
            item_unit_price: fd.item_unit_price,
            item_vat_rate: fd.item_vat_rate,
            payment_method: fd.payment_method,
            payment_deadline: fd.payment_deadline,
            bank_account: fd.bank_account.unwrap_or_default(),
        }
    }
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

#[derive(Clone, Deserialize)]
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
    // Single-item fields (backward compat from hidden fields)
    #[serde(default)]
    pub item_description: String,
    #[serde(default)]
    pub item_quantity: String,
    #[serde(default)]
    pub item_unit_price: String,
    #[serde(default)]
    pub item_vat_rate: String,
    pub payment_method: String,
    pub payment_deadline: String,
    pub bank_account: Option<String>,
}

#[derive(Deserialize)]
pub struct SubmitInvoiceForm {}

// --- Handlers ---

pub async fn list(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let nip_str = nip_ctx.account.nip.to_string();

    let filter = InvoiceFilter::for_account(nip_ctx.account.id.clone());
    let invoices = match state.invoice_service.list(&filter).await {
        Ok(invoices) => invoices,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udało się pobrać listy faktur: {err}"),
            )
                .into_response();
        }
    };

    let tab = params
        .get("tab")
        .cloned()
        .unwrap_or_else(|| "outgoing".to_string());

    let fetch_started = params.get("fetch").is_some_and(|v| v == "started");

    let (outgoing, incoming): (Vec<_>, Vec<_>) = invoices
        .into_iter()
        .partition(|inv| inv.direction == Direction::Outgoing);

    render(InvoiceListTemplate {
        active: "/invoices",
        nip_prefix: Some(nip_str),
        user_email: nip_ctx.user.email,
        outgoing,
        incoming,
        tab,
        fetch_started,
    })
}

fn new_form_defaults(nip_ctx: &NipContext, csrf_token: String) -> InvoiceNewTemplate {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let deadline = (chrono::Local::now() + chrono::Duration::days(14))
        .format("%Y-%m-%d")
        .to_string();
    InvoiceNewTemplate {
        active: "/invoices",
        nip_prefix: Some(nip_ctx.account.nip.to_string()),
        user_email: nip_ctx.user.email.clone(),
        error: None,
        f: InvoiceFormValues {
            invoice_number: String::new(),
            issue_date: today.clone(),
            sale_date: today,
            seller_nip: nip_ctx.account.nip.as_str().to_string(),
            seller_name: String::new(),
            seller_address_line1: String::new(),
            seller_address_line2: String::new(),
            buyer_nip: String::new(),
            buyer_name: String::new(),
            buyer_address_line1: String::new(),
            buyer_address_line2: String::new(),
            item_description: String::new(),
            item_quantity: "1".to_string(),
            item_unit_price: String::new(),
            item_vat_rate: "23".to_string(),
            payment_method: "transfer".to_string(),
            payment_deadline: deadline,
            bank_account: String::new(),
        },
        csrf_token,
    }
}

pub async fn new_form(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    session: Session,
) -> impl IntoResponse {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let mut tmpl = new_form_defaults(&nip_ctx, csrf_token);

    // Auto-fill seller data from Biała Lista cache
    if let Ok(info) = state
        .company_lookup_service
        .lookup(&nip_ctx.account.nip)
        .await
    {
        tmpl.f.seller_name = info.name;
        let (line1, line2) = split_address(&info.address);
        tmpl.f.seller_address_line1 = line1;
        tmpl.f.seller_address_line2 = line2;
        if let Some(account) = info.bank_accounts.first() {
            tmpl.f.bank_account = account.clone();
        }
    }

    render(tmpl)
}

/// Split "STREET, POSTCODE CITY" into (line1, line2).
fn split_address(address: &str) -> (String, String) {
    if let Some(pos) = address.rfind(", ") {
        let (line1, rest) = address.split_at(pos);
        (line1.to_string(), rest[2..].to_string())
    } else {
        (address.to_string(), String::new())
    }
}

pub async fn detail(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    session: Session,
    Path((_nip, id)): Path<(String, String)>,
) -> Response {
    let invoice_id: InvoiceId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Nieprawidłowy identyfikator faktury: {id}"),
            )
                .into_response();
        }
    };

    match state
        .invoice_service
        .find(&invoice_id, &nip_ctx.account.id)
        .await
    {
        Ok(invoice) => {
            let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
            render(InvoiceDetailTemplate {
                active: "/invoices",
                nip_prefix: Some(nip_ctx.account.nip.to_string()),
                user_email: nip_ctx.user.email,
                invoice,
                csrf_token,
            })
        }
        .into_response(),
        Err(err) => (
            status_for_service_error(&err),
            format!("Nie udało się pobrać faktury {invoice_id}: {err}"),
        )
            .into_response(),
    }
}

pub async fn create(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<InvoiceFormData>,
) -> Response {
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email.clone();
    let account_nip = nip_ctx.account.nip.clone();
    let account_id = nip_ctx.account.id.clone();
    let nip_str = account_nip.to_string();
    let form_values = InvoiceFormValues::from(form.clone());
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let mut input = match parse_form_to_input(form) {
        Ok(input) => input,
        Err(e) => {
            return render_with_status(
                StatusCode::BAD_REQUEST,
                InvoiceNewTemplate {
                    active: "/invoices",
                    nip_prefix: Some(nip_str),
                    user_email,
                    error: Some(e),
                    f: form_values,
                    csrf_token,
                },
            );
        }
    };

    // Auto-numbering: if invoice_number is empty, generate FV/YYYY/MM/NNN
    if input.invoice_number.trim().is_empty() {
        let year = input.issue_date.year();
        let month = input.issue_date.month();
        match state
            .invoice_sequence
            .next_number(&nip_ctx.account.nip, year, month)
            .await
        {
            Ok(seq) => {
                input.invoice_number = format_invoice_number("FV", year, month, seq);
            }
            Err(e) => {
                return render_with_status(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    InvoiceNewTemplate {
                        active: "/invoices",
                        nip_prefix: Some(nip_str),
                        user_email,
                        error: Some(format!("Błąd generowania numeru faktury: {e}")),
                        f: form_values,
                        csrf_token,
                    },
                );
            }
        }
    }

    match state
        .invoice_service
        .create_draft(input, account_id.clone())
        .await
    {
        Ok(invoice) => {
            log_audit_action(
                &state,
                &user_id,
                &user_email,
                Some(&account_nip),
                AuditAction::CreateInvoice,
                Some(format!(
                    "invoice_id={},invoice_number={}",
                    invoice.id, invoice.invoice_number
                )),
                client_ip(&headers),
            )
            .await;

            Redirect::to(&format!("/accounts/{nip_str}/invoices/{}", invoice.id)).into_response()
        }
        Err(e) => {
            let status = status_for_service_error(&e);
            render_with_status(
                status,
                InvoiceNewTemplate {
                    active: "/invoices",
                    nip_prefix: Some(nip_str),
                    user_email,
                    error: Some(format!("Nie udało się utworzyć faktury: {e}")),
                    f: form_values,
                    csrf_token,
                },
            )
        }
    }
}

pub async fn submit(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    Path((_nip, id)): Path<(String, String)>,
    CsrfForm(_form): CsrfForm<SubmitInvoiceForm>,
) -> Response {
    let nip_str = nip_ctx.account.nip.to_string();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email.clone();
    let account_nip = nip_ctx.account.nip.clone();
    let invoice_id: InvoiceId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Nieprawidłowy identyfikator faktury: {id}"),
            )
                .into_response();
        }
    };

    match state
        .invoice_service
        .submit(&invoice_id, &nip_ctx.account.id)
        .await
    {
        Ok(()) => {
            log_audit_action(
                &state,
                &user_id,
                &user_email,
                Some(&account_nip),
                AuditAction::SubmitInvoice,
                Some(format!("invoice_id={invoice_id}")),
                client_ip(&headers),
            )
            .await;

            Redirect::to(&format!("/accounts/{nip_str}/invoices/{invoice_id}")).into_response()
        }
        Err(err) => (
            status_for_service_error(&err),
            format!("Nie udało się wysłać faktury {invoice_id} do kolejki: {err}"),
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

    if form.item_description.is_empty() {
        return Err("Opis pozycji jest wymagany".to_string());
    }

    let line_items = parse_line_item(
        1,
        &form.item_description,
        &form.item_quantity,
        &form.item_unit_price,
        &form.item_vat_rate,
    )?;

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
        line_items: vec![line_items],
        payment_method,
        payment_deadline,
        bank_account: form.bank_account.filter(|s| !s.is_empty()),
    })
}

fn parse_line_item(
    line_number: u32,
    description: &str,
    quantity_raw: &str,
    unit_price_raw: &str,
    vat_rate_raw: &str,
) -> Result<LineItem, String> {
    let ctx = format!("Pozycja {line_number}");

    let quantity = Quantity::parse(quantity_raw).map_err(|e| format!("{ctx} — ilość: {e}"))?;
    let unit_price: Money = unit_price_raw
        .parse()
        .map_err(|e| format!("{ctx} — cena: {e}"))?;
    let vat_rate: VatRate = vat_rate_raw
        .parse()
        .map_err(|e| format!("{ctx} — stawka VAT: {e}"))?;

    let net = multiply_money_by_quantity(unit_price, &quantity)
        .map_err(|e| format!("{ctx} — wartosc netto: {e}"))?;
    let net_grosze = net.grosze();

    let vat_grosze = match vat_rate.percentage() {
        Some(pct) => {
            let vat_numerator = i128::from(net_grosze) * i128::from(pct);
            let rounded = div_round_half_away_from_zero(vat_numerator, 100);
            i64::try_from(rounded).map_err(|_| format!("{ctx} — wartosc VAT przekracza zakres"))?
        }
        None => 0,
    };

    Ok(LineItem {
        line_number,
        description: description.to_string(),
        unit: None,
        quantity,
        unit_net_price: Some(unit_price),
        net_value: net,
        vat_rate,
        vat_amount: Money::from_grosze(vat_grosze),
        gross_value: Money::from_grosze(net_grosze + vat_grosze),
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
        assert!(err.contains("ilość"), "unexpected error: {err}");
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
