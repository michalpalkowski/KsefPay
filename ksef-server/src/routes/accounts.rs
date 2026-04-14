use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;

use ksef_core::domain::nip::Nip;
use ksef_core::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};

use crate::extractors::AuthUser;
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

// --- Handlers ---

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Response {
    let accounts = match state.nip_account_repo.list_by_user(&auth.id).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udalo sie pobrac listy kont: {e}"),
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

pub async fn add_form(auth: AuthUser) -> Response {
    render(AccountAddTemplate {
        active: "/accounts",
        nip_prefix: None,
        user_email: auth.email,
        error: None,
        nip: String::new(),
        display_name: String::new(),
    })
}

pub async fn add(
    State(state): State<AppState>,
    auth: AuthUser,
    Form(form): Form<AddAccountFormData>,
) -> Response {
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
                    error: Some(format!("Nieprawidlowy NIP: {e}")),
                    nip: form_nip,
                    display_name: form_display_name,
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
            },
        );
    }

    // Check if NIP account already exists
    let account = match state.nip_account_repo.find_by_nip(&nip).await {
        Ok(Some(existing)) => existing,
        Ok(None) => {
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
                        error: Some(format!("Nie udalo sie utworzyc konta NIP: {e}")),
                        nip: form_nip,
                        display_name: form_display_name,
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
                    error: Some(format!("Blad serwera: {e}")),
                    nip: form_nip,
                    display_name: form_display_name,
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
