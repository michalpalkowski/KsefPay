use askama::Template;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;
use tracing::{error, info, warn};

use crate::csrf::ensure_csrf_token;
use crate::extractors::{CsrfForm, NipContext};
use crate::request_meta::client_ip;
use crate::state::{AppState, FetchJobStatus};
use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::session::{InvoiceQuery, SubjectType};

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
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "pages/fetch_history.html")]
struct FetchHistoryTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    error: Option<String>,
    rows: Vec<FetchHistoryRowView>,
    status_filter: String,
    q: String,
    page: u32,
    has_prev: bool,
    has_next: bool,
    prev_page: u32,
    next_page: u32,
}

#[derive(Clone)]
struct FetchHistoryRowView {
    timestamp: String,
    status_label: String,
    status_class: String,
    inserted: Option<u32>,
    updated: Option<u32>,
    error_count: usize,
    summary: String,
    errors: Vec<String>,
    message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchLogStatus {
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedFetchLog {
    pub status: FetchLogStatus,
    pub inserted: Option<u32>,
    pub updated: Option<u32>,
    pub error_count: u32,
    pub errors: Vec<String>,
    pub message: Option<String>,
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

#[derive(Deserialize)]
pub struct FetchHistoryQuery {
    pub page: Option<u32>,
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Serialize)]
struct FetchAuditDoneDetails {
    status: &'static str,
    inserted: u32,
    updated: u32,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct FetchAuditFailedDetails {
    status: &'static str,
    message: String,
}

#[derive(Deserialize)]
struct FetchAuditDetails {
    status: Option<String>,
    inserted: Option<u32>,
    updated: Option<u32>,
    errors: Option<Vec<String>>,
    message: Option<String>,
}

fn parse_legacy_fetch_metric(details: &str, key: &str) -> Option<u32> {
    details.split(',').find_map(|segment| {
        let (k, v) = segment.split_once('=')?;
        if k.trim() == key {
            v.trim().parse::<u32>().ok()
        } else {
            None
        }
    })
}

pub(crate) fn parse_fetch_log_details(details: &str) -> Option<ParsedFetchLog> {
    if let Ok(parsed) = serde_json::from_str::<FetchAuditDetails>(details) {
        match parsed.status.as_deref() {
            Some("failed") => {
                let message = parsed
                    .message
                    .unwrap_or_else(|| "Pobieranie nie powiodlo sie".to_string());
                return Some(ParsedFetchLog {
                    status: FetchLogStatus::Failed,
                    inserted: None,
                    updated: None,
                    error_count: 0,
                    errors: vec![],
                    message: Some(message),
                });
            }
            Some("done") | None => {
                if let (Some(inserted), Some(updated)) = (parsed.inserted, parsed.updated) {
                    let errors = parsed.errors.unwrap_or_default();
                    return Some(ParsedFetchLog {
                        status: FetchLogStatus::Done,
                        inserted: Some(inserted),
                        updated: Some(updated),
                        error_count: errors.len() as u32,
                        errors,
                        message: None,
                    });
                }
            }
            Some(_) => {}
        }
    }

    // Backward compatibility with old detail format:
    // "inserted=43,updated=18,errors=2"
    let inserted = parse_legacy_fetch_metric(details, "inserted")?;
    let updated = parse_legacy_fetch_metric(details, "updated")?;
    let error_count = parse_legacy_fetch_metric(details, "errors").unwrap_or(0);
    Some(ParsedFetchLog {
        status: FetchLogStatus::Done,
        inserted: Some(inserted),
        updated: Some(updated),
        error_count,
        errors: vec![],
        message: None,
    })
}

// --- Handlers ---

pub async fn fetch_form(nip_ctx: NipContext, session: Session) -> impl IntoResponse {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let month_ago = (chrono::Local::now() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    render(FetchFormTemplate {
        active: "/fetch",
        nip_prefix: Some(nip_ctx.account.nip.to_string()),
        user_email: nip_ctx.user.email,
        error: None,
        default_date_from: month_ago,
        default_date_to: today,
        csrf_token,
    })
}

const FETCH_HISTORY_SCAN_LIMIT: u32 = 2_000;
const FETCH_HISTORY_PAGE_SIZE: usize = 20;

pub async fn fetch_history(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    Query(query): Query<FetchHistoryQuery>,
) -> impl IntoResponse {
    let mut page = query.page.unwrap_or(1).max(1);
    let status_filter = query
        .status
        .as_deref()
        .map(str::to_ascii_lowercase)
        .filter(|v| matches!(v.as_str(), "all" | "done" | "failed"))
        .unwrap_or_else(|| "all".to_string());
    let q = query.q.unwrap_or_default().trim().to_string();
    let q_lower = q.to_ascii_lowercase();

    let entries = match state
        .audit_service
        .list_recent(FETCH_HISTORY_SCAN_LIMIT)
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return render(FetchHistoryTemplate {
                active: "/fetch-history",
                nip_prefix: Some(nip_ctx.account.nip.to_string()),
                user_email: nip_ctx.user.email,
                error: Some(format!("Nie udalo sie odczytac historii pobran: {err}")),
                rows: vec![],
                status_filter,
                q,
                page: 1,
                has_prev: false,
                has_next: false,
                prev_page: 1,
                next_page: 1,
            });
        }
    };

    let mut rows: Vec<FetchHistoryRowView> = entries
        .into_iter()
        .filter(|entry| {
            entry.action == AuditAction::FetchInvoices
                && entry
                    .nip
                    .as_ref()
                    .is_some_and(|entry_nip| entry_nip == &nip_ctx.account.nip)
        })
        .filter_map(|entry| {
            let details = entry.details.as_deref()?;
            let parsed = parse_fetch_log_details(details)?;

            let has_errors = parsed.error_count > 0;
            let (status_label, status_class) = match parsed.status {
                FetchLogStatus::Done if has_errors => ("Zakonczone z bledami", "error"),
                FetchLogStatus::Done => ("Zakonczone", "success"),
                FetchLogStatus::Failed => ("Niepowodzenie", "error"),
            };

            let summary = match parsed.status {
                FetchLogStatus::Done => {
                    let inserted = parsed.inserted.unwrap_or(0);
                    let updated = parsed.updated.unwrap_or(0);
                    if parsed.error_count == 0 {
                        format!("Pobrano: {inserted} nowych, {updated} zaktualizowanych.")
                    } else {
                        format!(
                            "Pobrano: {inserted} nowych, {updated} zaktualizowanych. Bledy: {}.",
                            parsed.error_count
                        )
                    }
                }
                FetchLogStatus::Failed => parsed
                    .message
                    .clone()
                    .unwrap_or_else(|| "Pobieranie nie powiodlo sie".to_string()),
            };

            Some(FetchHistoryRowView {
                timestamp: entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                status_label: status_label.to_string(),
                status_class: status_class.to_string(),
                inserted: parsed.inserted,
                updated: parsed.updated,
                error_count: parsed.error_count as usize,
                summary,
                errors: parsed.errors,
                message: parsed.message,
            })
        })
        .collect();

    if status_filter != "all" {
        rows.retain(|row| {
            (status_filter == "done" && row.status_class == "success")
                || (status_filter == "failed" && row.status_class == "error")
        });
    }
    if !q_lower.is_empty() {
        rows.retain(|row| {
            row.summary.to_ascii_lowercase().contains(&q_lower)
                || row
                    .errors
                    .iter()
                    .any(|err| err.to_ascii_lowercase().contains(&q_lower))
                || row
                    .message
                    .as_ref()
                    .is_some_and(|msg| msg.to_ascii_lowercase().contains(&q_lower))
        });
    }

    let total = rows.len();
    let total_pages = if total == 0 {
        1
    } else {
        total.div_ceil(FETCH_HISTORY_PAGE_SIZE) as u32
    };
    if page > total_pages {
        page = total_pages;
    }

    let start = ((page - 1) as usize) * FETCH_HISTORY_PAGE_SIZE;
    let end = (start + FETCH_HISTORY_PAGE_SIZE).min(total);
    let paged_rows = if start < end {
        rows[start..end].to_vec()
    } else {
        vec![]
    };

    render(FetchHistoryTemplate {
        active: "/fetch-history",
        nip_prefix: Some(nip_ctx.account.nip.to_string()),
        user_email: nip_ctx.user.email,
        error: None,
        rows: paged_rows,
        status_filter,
        q,
        page,
        has_prev: page > 1,
        has_next: page < total_pages,
        prev_page: page.saturating_sub(1).max(1),
        next_page: page + 1,
    })
}

pub async fn fetch_execute(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<FetchFormData>,
) -> Response {
    let nip_str = nip_ctx.account.nip.to_string();
    let account_id = nip_ctx.account.id.clone();
    let account_nip = nip_ctx.account.nip.clone();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let ip_address = client_ip(&headers);

    let date_from = match NaiveDate::parse_from_str(&form.date_from, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return render_form_error(nip_str, user_email, format!("Data od: {e}"), csrf_token);
        }
    };
    let date_to = match NaiveDate::parse_from_str(&form.date_to, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return render_form_error(nip_str, user_email, format!("Data do: {e}"), csrf_token);
        }
    };
    if date_from > date_to {
        return render_form_error(
            nip_str,
            user_email,
            "Data od musi być mniejsza lub równa dacie do".to_string(),
            csrf_token,
        );
    }

    let subject_type: SubjectType = match form.subject_type.parse() {
        Ok(st) => st,
        Err(e) => {
            return render_form_error(
                nip_str,
                user_email,
                format!("Typ podmiotu: {e}"),
                csrf_token,
            );
        }
    };

    let query = InvoiceQuery {
        date_from,
        date_to,
        subject_type,
    };

    // Prevent duplicate background jobs for the same account.
    {
        let mut jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
        if matches!(jobs.get(&account_id), Some(FetchJobStatus::Running)) {
            return render_form_error(
                nip_str,
                user_email,
                "Pobieranie dla tego konta już trwa. Poczekaj na zakończenie.".to_string(),
                csrf_token,
            );
        }
        jobs.insert(account_id.clone(), FetchJobStatus::Running);
    }

    let nip = nip_ctx.account.nip.clone();
    let fetch_service = state.fetch_service.clone();
    let audit_service = state.audit_service.clone();
    let fetch_jobs = state.fetch_jobs.clone();
    let job_key = account_id.clone();
    let audit_user_id = user_id;
    let audit_user_email = user_email.clone();
    let audit_nip = account_nip;
    let audit_ip = ip_address;

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
                for item in &result.errors {
                    warn!(
                        nip = %nip,
                        ksef_number = %item.ksef_number,
                        error = %item.error,
                        "Fetch item failed"
                    );
                }
                let error_msgs: Vec<String> = result
                    .errors
                    .iter()
                    .map(|e| format!("{}: {}", e.ksef_number, e.error))
                    .collect();
                {
                    let mut jobs = fetch_jobs.lock().expect("fetch_jobs lock");
                    jobs.insert(
                        job_key,
                        FetchJobStatus::Done {
                            inserted: result.inserted,
                            updated: result.updated,
                            errors: error_msgs.clone(),
                        },
                    );
                }

                if let Err(err) = audit_service
                    .log_action(
                        &audit_user_id,
                        &audit_user_email,
                        Some(&audit_nip),
                        AuditAction::FetchInvoices,
                        serde_json::to_string(&FetchAuditDoneDetails {
                            status: "done",
                            inserted: result.inserted,
                            updated: result.updated,
                            errors: error_msgs.clone(),
                        })
                        .ok(),
                        audit_ip.clone(),
                    )
                    .await
                {
                    warn!(error = %err, user_id = %audit_user_id, "failed to write audit log");
                }
            }
            Err(e) => {
                error!(nip = %nip, error = %e, "Fetch failed");
                {
                    let mut jobs = fetch_jobs.lock().expect("fetch_jobs lock");
                    jobs.insert(job_key, FetchJobStatus::Failed(e.to_string()));
                }

                if let Err(err) = audit_service
                    .log_action(
                        &audit_user_id,
                        &audit_user_email,
                        Some(&audit_nip),
                        AuditAction::FetchInvoices,
                        serde_json::to_string(&FetchAuditFailedDetails {
                            status: "failed",
                            message: e.to_string(),
                        })
                        .ok(),
                        audit_ip.clone(),
                    )
                    .await
                {
                    warn!(error = %err, user_id = %audit_user_id, "failed to write audit log");
                }
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
    let account_id = nip_ctx.account.id.clone();
    let status = {
        let jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
        jobs.get(&account_id).cloned()
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
            {
                let mut jobs = state.fetch_jobs.lock().expect("fetch_jobs lock");
                jobs.remove(&account_id);
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
                jobs.remove(&account_id);
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

fn render_form_error(
    nip_prefix: String,
    user_email: String,
    error: String,
    csrf_token: String,
) -> Response {
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
        csrf_token,
    })
}
