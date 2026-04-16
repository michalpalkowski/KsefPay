use std::time::Duration;

use askama::Template;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use chrono::NaiveDate;
use serde::Deserialize;
use tower_sessions::Session;

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::session::{InvoiceQuery, SubjectType};

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::{CsrfForm, NipContext};
use crate::request_meta::client_ip;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/export.html")]
struct ExportTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    error: Option<String>,
    result: Option<ExportResultDisplay>,
    default_date_from: String,
    default_date_to: String,
    csrf_token: String,
}

#[allow(dead_code)]
struct ExportResultDisplay {
    reference: String,
    status: String,
    status_class: String,
    download_url: Option<String>,
    is_pending: bool,
    can_download: bool,
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

fn default_dates() -> (String, String) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let month_ago = (chrono::Local::now() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    (month_ago, today)
}

fn status_display(status: ksef_core::domain::export::ExportStatus) -> (String, String) {
    match status {
        ksef_core::domain::export::ExportStatus::Pending => {
            ("W trakcie przygotowania".to_string(), "queued".to_string())
        }
        ksef_core::domain::export::ExportStatus::Completed => {
            ("Gotowy do pobrania".to_string(), "accepted".to_string())
        }
        ksef_core::domain::export::ExportStatus::Failed => {
            ("Błąd eksportu".to_string(), "failed".to_string())
        }
    }
}

fn form_error(
    nip_prefix: String,
    user_email: String,
    error: String,
    csrf_token: String,
) -> Response {
    let (def_from, def_to) = default_dates();
    render(ExportTemplate {
        active: "/export",
        nip_prefix: Some(nip_prefix),
        user_email,
        error: Some(error),
        result: None,
        default_date_from: def_from,
        default_date_to: def_to,
        csrf_token,
    })
}

#[derive(Deserialize)]
pub struct ExportFormData {
    pub date_from: String,
    pub date_to: String,
    pub subject_type: String,
}

pub async fn export_page(nip_ctx: NipContext, session: Session) -> Response {
    let (from, to) = default_dates();
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    render(ExportTemplate {
        active: "/export",
        nip_prefix: Some(nip_ctx.account.nip.to_string()),
        user_email: nip_ctx.user.email,
        error: None,
        result: None,
        default_date_from: from,
        default_date_to: to,
        csrf_token,
    })
}

pub async fn start_export(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<ExportFormData>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let user_id = nip_ctx.user.id.clone();
    let account_id = nip_ctx.account.id.clone();
    let nip_str = nip.to_string();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let date_from = match NaiveDate::parse_from_str(&form.date_from, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => return form_error(nip_str, user_email, format!("Data od: {e}"), csrf_token),
    };
    let date_to = match NaiveDate::parse_from_str(&form.date_to, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => return form_error(nip_str, user_email, format!("Data do: {e}"), csrf_token),
    };
    let subject_type: SubjectType = match form.subject_type.parse() {
        Ok(s) => s,
        Err(e) => {
            return form_error(
                nip_str,
                user_email,
                format!("Typ podmiotu: {e}"),
                csrf_token,
            );
        }
    };

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return form_error(
                nip_str,
                user_email,
                format!("Brak tokenu dostępu: {e}"),
                csrf_token,
            );
        }
    };

    let query = InvoiceQuery {
        date_from,
        date_to,
        subject_type,
    };

    let job = match state.export_service.start(&token, query).await {
        Ok(j) => j,
        Err(e) => {
            return form_error(
                nip_str,
                user_email,
                format!("Eksport nie powiódł się: {e}"),
                csrf_token,
            );
        }
    };

    log_audit_action(
        &state,
        &user_id,
        &user_email,
        Some(nip),
        AuditAction::ExportStart,
        Some(format!(
            "reference={},date_from={},date_to={},subject_type={}",
            job.reference_number, form.date_from, form.date_to, form.subject_type
        )),
        client_ip(&headers),
    )
    .await;

    // Store encryption key for later download
    if let (Some(key), Some(iv)) = (&job.encryption_key, &job.encryption_iv) {
        state.export_keys.lock().unwrap().insert(
            (account_id.clone(), job.reference_number.clone()),
            (key.clone(), iv.clone()),
        );
    }

    // Poll for up to ~15s to see if it completes quickly
    let result = state
        .export_service
        .wait_until_complete(&token, &job.reference_number, 5, Duration::from_secs(3))
        .await;

    let (def_from, def_to) = default_dates();
    match result {
        Ok(completed) => {
            let (status_text, status_class) = status_display(completed.status);
            let has_key = state
                .export_keys
                .lock()
                .unwrap()
                .contains_key(&(account_id.clone(), completed.reference_number.clone()));
            render(ExportTemplate {
                active: "/export",
                nip_prefix: Some(nip_str),
                user_email,
                error: None,
                result: Some(ExportResultDisplay {
                    reference: completed.reference_number,
                    status: status_text,
                    status_class,
                    download_url: completed.download_url,
                    is_pending: false,
                    can_download: has_key,
                }),
                default_date_from: def_from,
                default_date_to: def_to,
                csrf_token: csrf_token.clone(),
            })
        }
        Err(_) => render(ExportTemplate {
            active: "/export",
            nip_prefix: Some(nip_str),
            user_email,
            error: None,
            result: Some(ExportResultDisplay {
                reference: job.reference_number,
                status: "W trakcie przygotowania".to_string(),
                status_class: "queued".to_string(),
                download_url: None,
                is_pending: true,
                can_download: true,
            }),
            default_date_from: def_from,
            default_date_to: def_to,
            csrf_token,
        }),
    }
}

pub async fn check_status(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    session: Session,
    Path((_nip, reference)): Path<(String, String)>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let account_id = nip_ctx.account.id.clone();
    let nip_str = nip.to_string();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return form_error(
                nip_str,
                user_email,
                format!("Brak tokenu dostępu: {e}"),
                csrf_token,
            );
        }
    };

    let (def_from, def_to) = default_dates();
    match state.export_service.get_status(&token, &reference).await {
        Ok(job) => {
            let (status_text, status_class) = status_display(job.status);
            let is_pending = matches!(job.status, ksef_core::domain::export::ExportStatus::Pending);
            let has_key = state
                .export_keys
                .lock()
                .unwrap()
                .contains_key(&(account_id.clone(), reference.clone()));
            render(ExportTemplate {
                active: "/export",
                nip_prefix: Some(nip_str),
                user_email,
                error: None,
                result: Some(ExportResultDisplay {
                    reference: job.reference_number,
                    status: status_text,
                    status_class,
                    download_url: job.download_url,
                    is_pending,
                    can_download: has_key && !is_pending,
                }),
                default_date_from: def_from,
                default_date_to: def_to,
                csrf_token,
            })
        }
        Err(e) => form_error(
            nip_str,
            user_email,
            format!("Sprawdzenie statusu nie powiodło się: {e}"),
            csrf_token,
        ),
    }
}

/// Download the export file, decrypt it, and serve as plain ZIP.
pub async fn download(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    Path((_nip, reference)): Path<(String, String)>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let account_id = nip_ctx.account.id.clone();

    let key_ref = (account_id.clone(), reference.clone());
    let (key, iv) = match state.export_keys.lock().unwrap().get(&key_ref) {
        Some((k, i)) => (k.clone(), i.clone()),
        None => {
            return (
                StatusCode::NOT_FOUND,
                "Klucz deszyfrowania nie jest dostępny. Rozpocznij nowy eksport.",
            )
                .into_response();
        }
    };

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Brak tokenu dostępu: {e}"),
            )
                .into_response();
        }
    };

    // Get the download URL from KSeF
    let job = match state.export_service.get_status(&token, &reference).await {
        Ok(j) => j,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Błąd statusu eksportu: {e}"),
            )
                .into_response();
        }
    };

    let Some(download_url) = job.download_url else {
        return (
            StatusCode::BAD_REQUEST,
            "Eksport nie jest jeszcze gotowy do pobrania.",
        )
            .into_response();
    };

    match state
        .export_service
        .download_and_decrypt(&download_url, &key, &iv)
        .await
    {
        Ok(zip_bytes) => {
            // Clean up stored key
            state.export_keys.lock().unwrap().remove(&key_ref);

            let filename = format!("ksef-export-{reference}.zip");
            (
                [
                    (header::CONTENT_TYPE, "application/zip"),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("attachment; filename=\"{filename}\""),
                    ),
                ],
                Body::from(zip_bytes),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Deszyfrowanie nie powiodło się: {e}"),
        )
            .into_response(),
    }
}
