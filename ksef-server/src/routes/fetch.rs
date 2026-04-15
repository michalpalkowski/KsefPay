use askama::Template;
use axum::Form;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use ksef_core::domain::session::{InvoiceQuery, SubjectType};

use crate::extractors::NipContext;
use crate::state::{AppState, FetchJobStatus};

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

// --- JSON response for polling ---

#[derive(Serialize)]
pub struct FetchStatusResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
            "Data od musi być mniejsza lub równa dacie do".to_string(),
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

    // Mark as running
    {
        let mut jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
        jobs.insert(nip_str.clone(), FetchJobStatus::Running);
    }

    let nip = nip_ctx.account.nip.clone();
    let account_id = nip_ctx.account.id.clone();
    let fetch_service = state.fetch_service.clone();
    let fetch_jobs = state.fetch_jobs.clone();
    let job_key = nip_str.clone();

    tokio::spawn(async move {
        match fetch_service
            .fetch_invoices(&nip, &account_id, &query)
            .await
        {
            Ok(result) => {
                info!(
                    nip = %nip,
                    inserted = result.inserted,
                    updated = result.updated,
                    errors = result.errors.len(),
                    "Fetch completed"
                );
                let error_msgs: Vec<String> = result
                    .errors
                    .iter()
                    .map(|e| format!("{}: {}", e.ksef_number, e.error))
                    .collect();
                let mut jobs = fetch_jobs.lock().expect("fetch_jobs lock");
                jobs.insert(
                    job_key,
                    FetchJobStatus::Done {
                        inserted: result.inserted,
                        updated: result.updated,
                        errors: error_msgs,
                    },
                );
            }
            Err(e) => {
                error!(nip = %nip, error = %e, "Fetch failed");
                let mut jobs = fetch_jobs.lock().expect("fetch_jobs lock");
                jobs.insert(job_key, FetchJobStatus::Failed(e.to_string()));
            }
        }
    });

    Redirect::to(&format!("/accounts/{nip_str}/invoices?fetch=started")).into_response()
}

/// JSON endpoint for polling fetch job status.
pub async fn fetch_status(
    State(state): State<AppState>,
    nip_ctx: NipContext,
) -> Json<FetchStatusResponse> {
    let nip_str = nip_ctx.account.nip.to_string();
    let status = {
        let jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
        jobs.get(&nip_str).cloned()
    };

    match status {
        Some(FetchJobStatus::Running) => Json(FetchStatusResponse {
            status: "running".to_string(),
            inserted: None,
            updated: None,
            errors: None,
            message: None,
        }),
        Some(FetchJobStatus::Done {
            inserted,
            updated,
            errors,
        }) => {
            // Clean up after reading
            {
                let mut jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
                jobs.remove(&nip_str);
            }
            Json(FetchStatusResponse {
                status: "done".to_string(),
                inserted: Some(inserted),
                updated: Some(updated),
                errors: if errors.is_empty() {
                    None
                } else {
                    Some(errors)
                },
                message: None,
            })
        }
        Some(FetchJobStatus::Failed(msg)) => {
            {
                let mut jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
                jobs.remove(&nip_str);
            }
            Json(FetchStatusResponse {
                status: "failed".to_string(),
                inserted: None,
                updated: None,
                errors: None,
                message: Some(msg),
            })
        }
        None => Json(FetchStatusResponse {
            status: "none".to_string(),
            inserted: None,
            updated: None,
            errors: None,
            message: None,
        }),
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
