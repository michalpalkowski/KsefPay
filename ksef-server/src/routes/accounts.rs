use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use std::time::Duration;
use tower_sessions::Session;

use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::nip::Nip;
use ksef_core::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
use ksef_core::domain::session::{InvoiceQuery, SubjectType};
use ksef_core::infra::ksef::testdata::TestSubjectType;
use ksef_core::infra::ksef::{KSeFApiClient, TestDataClient};
use ksef_core::ports::ksef_client::KSeFClient;

use crate::csrf::ensure_csrf_token;
use crate::extractors::{AuthUser, CsrfForm};
use crate::state::AppState;

// --- Templates ---

#[derive(Template)]
#[template(path = "pages/accounts.html")]
struct AccountsTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    accounts: Vec<NipAccount>,
}

#[derive(Template)]
#[template(path = "pages/account_add.html")]
struct AccountAddTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    error: Option<String>,
    nip: String,
    display_name: String,
    csrf_token: String,
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

// --- Form data ---

#[derive(Deserialize)]
pub struct AddAccountFormData {
    pub nip: String,
    pub display_name: String,
}

const BOOTSTRAP_MAX_ATTEMPTS: u32 = 3;
const BOOTSTRAP_RETRY_DELAY: Duration = Duration::from_millis(1_500);

async fn verify_invoice_query_access(
    state: &AppState,
    access_token: &ksef_core::domain::auth::AccessToken,
) -> Result<(), String> {
    let client = KSeFApiClient::new(state.ksef_environment);
    let today = Utc::now().date_naive();
    let query = InvoiceQuery {
        date_from: today,
        date_to: today,
        subject_type: SubjectType::Subject1,
    };

    client
        .query_invoices(access_token, &query)
        .await
        .map(|_| ())
        .map_err(|e| format!("brak dostępu do zapytań faktur KSeF (InvoiceRead): {e}"))
}

async fn bootstrap_sandbox_subject(
    state: &AppState,
    nip: &Nip,
    ensure_subject_exists: bool,
) -> Result<(), String> {
    if ensure_subject_exists {
        let client = TestDataClient::new(state.ksef_environment);
        let subject_result = client
            .create_subject(
                nip,
                &format!("ksef-paymoney test subject NIP {nip}"),
                TestSubjectType::EnforcementAuthority,
            )
            .await
            .map_err(|e| format!("rejestracja podmiotu testowego nie powiodła się: {e}"))?;
        tracing::info!(nip = %nip, ?subject_result, "sandbox subject registration result");
    }

    let token_pair = state
        .session_service
        .ensure_token(nip)
        .await
        .map_err(|e| format!("nie udało się uwierzytelnić NIP w KSeF: {e}"))?;

    for attempt in 1..=BOOTSTRAP_MAX_ATTEMPTS {
        match verify_invoice_query_access(state, &token_pair.access_token).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                tracing::warn!(
                    nip = %nip,
                    attempt,
                    max_attempts = BOOTSTRAP_MAX_ATTEMPTS,
                    error = %err,
                    "sandbox invoice access check failed"
                );
                if attempt < BOOTSTRAP_MAX_ATTEMPTS {
                    tokio::time::sleep(BOOTSTRAP_RETRY_DELAY).await;
                } else {
                    return Err(err);
                }
            }
        }
    }

    Err("nie udało się potwierdzić dostępu do zapytań faktur KSeF".to_string())
}

// --- Handlers ---

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Response {
    let accounts = match state.nip_account_repo.list_by_user(&auth.id).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udało się pobrać listy kont: {e}"),
            )
                .into_response();
        }
    };

    render(AccountsTemplate {
        active: "/accounts",
        nip_prefix: None,
        user_email: auth.email,
        accounts,
    })
}

pub async fn add_form(auth: AuthUser, session: Session) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    render(AccountAddTemplate {
        active: "/accounts",
        nip_prefix: None,
        user_email: auth.email,
        error: None,
        nip: String::new(),
        display_name: String::new(),
        csrf_token,
    })
}

pub async fn add(
    State(state): State<AppState>,
    auth: AuthUser,
    session: Session,
    CsrfForm(form): CsrfForm<AddAccountFormData>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let form_nip = form.nip.clone();
    let form_display_name = form.display_name.clone();

    let nip = match Nip::parse(&form.nip) {
        Ok(n) => n,
        Err(e) => {
            return render_with_status(
                StatusCode::BAD_REQUEST,
                AccountAddTemplate {
                    active: "/accounts",
                    nip_prefix: None,
                    user_email: auth.email,
                    error: Some(format!("Nieprawidłowy NIP: {e}")),
                    nip: form_nip,
                    display_name: form_display_name,
                    csrf_token,
                },
            );
        }
    };

    let display_name = form.display_name.trim().to_string();
    if display_name.is_empty() {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            AccountAddTemplate {
                active: "/accounts",
                nip_prefix: None,
                user_email: auth.email,
                error: Some("Nazwa wyswietlana jest wymagana".to_string()),
                nip: form_nip,
                display_name: form_display_name,
                csrf_token,
            },
        );
    }

    // Check if NIP account already exists
    let account = match state.nip_account_repo.find_by_nip(&nip).await {
        Ok(Some(existing)) => {
            if state.ksef_environment != KSeFEnvironment::Production
                && let Err(e) = bootstrap_sandbox_subject(&state, &existing.nip, false).await
            {
                return render_with_status(
                    StatusCode::BAD_GATEWAY,
                    AccountAddTemplate {
                        active: "/accounts",
                        nip_prefix: None,
                        user_email: auth.email,
                        error: Some(format!(
                            "Konto NIP istnieje, ale nie ma wymaganych uprawnień KSeF sandbox: {e}"
                        )),
                        nip: form_nip,
                        display_name: form_display_name,
                        csrf_token,
                    },
                );
            }
            existing
        }
        Ok(None) => {
            if state.ksef_environment != KSeFEnvironment::Production
                && let Err(e) = bootstrap_sandbox_subject(&state, &nip, true).await
            {
                return render_with_status(
                    StatusCode::BAD_GATEWAY,
                    AccountAddTemplate {
                        active: "/accounts",
                        nip_prefix: None,
                        user_email: auth.email,
                        error: Some(format!(
                            "Nie udało się skonfigurować konta w KSeF sandbox: {e}"
                        )),
                        nip: form_nip,
                        display_name: form_display_name,
                        csrf_token,
                    },
                );
            }

            // Create new NIP account
            let now = Utc::now();
            let account = NipAccount {
                id: NipAccountId::new(),
                nip,
                display_name,
                ksef_auth_method: KSeFAuthMethod::Xades,
                ksef_auth_token: None,
                cert_pem: None,
                key_pem: None,
                cert_auto_generated: false,
                created_at: now,
                updated_at: now,
            };
            if let Err(e) = state.nip_account_repo.create(&account).await {
                return render_with_status(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AccountAddTemplate {
                        active: "/accounts",
                        nip_prefix: None,
                        user_email: auth.email,
                        error: Some(format!("Nie udało się utworzyć konta NIP: {e}")),
                        nip: form_nip,
                        display_name: form_display_name,
                        csrf_token,
                    },
                );
            }
            account
        }
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                AccountAddTemplate {
                    active: "/accounts",
                    nip_prefix: None,
                    user_email: auth.email,
                    error: Some(format!("Błąd serwera: {e}")),
                    nip: form_nip,
                    display_name: form_display_name,
                    csrf_token,
                },
            );
        }
    };

    // Grant access to the current user
    if let Err(e) = state
        .nip_account_repo
        .grant_access(&auth.id, &account.id)
        .await
    {
        // Duplicate access grant is not an error -- the user might already have access
        tracing::warn!(
            user_id = %auth.id,
            account_id = %account.id,
            "grant_access error (may be duplicate): {e}"
        );
    }

    Redirect::to("/accounts").into_response()
}
