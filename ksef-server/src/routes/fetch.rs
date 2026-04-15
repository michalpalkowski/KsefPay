use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use chrono::NaiveDate;
use serde::Deserialize;

use ksef_core::domain::session::{InvoiceQuery, SubjectType};

use crate::extractors::NipContext;
use crate::state::AppState;

// --- Templates ---

#[derive(Template)]
#[template(path = "pages/fetch.html")]
struct FetchFormTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    error: Option<String>,
    default_date_from: String,
    default_date_to: String,
}

#[derive(Template)]
#[template(path = "pages/fetch_results.html")]
struct FetchResultsTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    inserted: u32,
    updated: u32,
    errors: Vec<FetchErrorDisplay>,
}

struct FetchErrorDisplay {
    ksef_number: String,
    error: String,
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

// --- Form data ---

#[derive(Deserialize)]
pub struct FetchFormData {
    pub date_from: String,
    pub date_to: String,
    pub subject_type: String,
}

// --- Handlers ---

pub async fn fetch_form(nip_ctx: NipContext) -> impl IntoResponse {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let month_ago = (chrono::Local::now() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    render(FetchFormTemplate {
        active: "/fetch",
        nip_prefix: Some(nip_ctx.account.nip.to_string()),
        user_email: nip_ctx.user.email,
        error: None,
        default_date_from: month_ago,
        default_date_to: today,
    })
}

pub async fn fetch_execute(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    Form(form): Form<FetchFormData>,
) -> Response {
    let nip_str = nip_ctx.account.nip.to_string();
    let user_email = nip_ctx.user.email;

    let date_from = match NaiveDate::parse_from_str(&form.date_from, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return render_form_error(nip_str, user_email, format!("Data od: {e}"));
        }
    };
    let date_to = match NaiveDate::parse_from_str(&form.date_to, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return render_form_error(nip_str, user_email, format!("Data do: {e}"));
        }
    };
    if date_from > date_to {
        return render_form_error(
            nip_str,
            user_email,
            "Data od musi byc mniejsza lub rowna dacie do".to_string(),
        );
    }

    let subject_type: SubjectType = match form.subject_type.parse() {
        Ok(st) => st,
        Err(e) => {
            return render_form_error(nip_str, user_email, format!("Typ podmiotu: {e}"));
        }
    };

    let query = InvoiceQuery {
        date_from,
        date_to,
        subject_type,
    };

    match state.fetch_service.fetch_invoices(&nip_ctx.account.nip, &query).await {
        Ok(result) => {
            let errors: Vec<FetchErrorDisplay> = result
                .errors
                .into_iter()
                .map(|e| FetchErrorDisplay {
                    ksef_number: e.ksef_number.to_string(),
                    error: e.error.to_string(),
                })
                .collect();

            render(FetchResultsTemplate {
                active: "/fetch",
                nip_prefix: Some(nip_str),
                user_email,
                inserted: result.inserted,
                updated: result.updated,
                errors,
            })
        }
        Err(e) => render_form_error(
            nip_str,
            user_email,
            format!("Pobieranie nie powiodlo sie: {e}"),
        ),
    }
}

fn render_form_error(nip_prefix: String, user_email: String, error: String) -> Response {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let month_ago = (chrono::Local::now() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    render(FetchFormTemplate {
        active: "/fetch",
        nip_prefix: Some(nip_prefix),
        user_email,
        error: Some(error),
        default_date_from: month_ago,
        default_date_to: today,
    })
}
