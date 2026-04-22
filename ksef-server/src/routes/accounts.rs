use askama::Template;
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use openssl::pkey::PKey;
use openssl::x509::X509;
use serde::Deserialize;
use std::time::Duration;
use tower_sessions::Session;

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::nip::Nip;
use ksef_core::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
use ksef_core::domain::session::{InvoiceQuery, SubjectType};
use ksef_core::infra::ksef::testdata::TestSubjectType;
use ksef_core::infra::ksef::{KSeFApiClient, TestDataClient};
use ksef_core::ports::ksef_client::KSeFClient;

use crate::audit_log;
use crate::csrf::{CSRF_SESSION_KEY, ensure_csrf_token};
use crate::extractors::{AuthUser, CsrfForm};
use crate::request_meta::client_ip;
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

#[derive(Template)]
#[template(path = "pages/account_certificate.html")]
struct AccountCertificateTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    account: NipAccount,
    error: Option<String>,
    success: Option<String>,
    csrf_token: String,
    has_custom_certificate: bool,
    can_manage_credentials: bool,
    environment_name: &'static str,
    runtime_status: String,
    certificate_pem: String,
    private_key_pem: String,
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

fn build_certificate_template(
    account: NipAccount,
    user_email: String,
    environment: KSeFEnvironment,
    can_manage_credentials: bool,
    error: Option<String>,
    success: Option<String>,
    certificate_pem: String,
    private_key_pem: String,
    csrf_token: String,
) -> AccountCertificateTemplate {
    let has_custom_certificate = account.cert_pem.is_some() && account.key_pem.is_some();
    let environment_name = match environment {
        KSeFEnvironment::Test => "test",
        KSeFEnvironment::Demo => "demo",
        KSeFEnvironment::Production => "production",
    };
    let runtime_status = match (environment, has_custom_certificate) {
        (_, true) => {
            "Przy następnym uwierzytelnieniu XAdES aplikacja użyje zapisanej pary certyfikat + klucz dla tego NIP."
                .to_string()
        }
        (KSeFEnvironment::Production, false) => {
            "Brak zapisanej pary certyfikat + klucz. W production logowanie XAdES dla tego NIP nie powiedzie się, dopóki ich nie dodasz."
                .to_string()
        }
        (_, false) => {
            "Brak zapisanej pary certyfikat + klucz. W środowisku test/demo aplikacja wygeneruje self-signed dla tego NIP przy uwierzytelnieniu."
                .to_string()
        }
    };

    AccountCertificateTemplate {
        active: "/certificate",
        nip_prefix: Some(account.nip.to_string()),
        user_email,
        account,
        error,
        success,
        csrf_token,
        has_custom_certificate,
        can_manage_credentials,
        environment_name,
        runtime_status,
        certificate_pem,
        private_key_pem,
    }
}

fn render_certificate_page(
    account: NipAccount,
    user_email: String,
    environment: KSeFEnvironment,
    can_manage_credentials: bool,
    error: Option<String>,
    success: Option<String>,
    certificate_pem: String,
    private_key_pem: String,
    csrf_token: String,
) -> Response {
    render(build_certificate_template(
        account,
        user_email,
        environment,
        can_manage_credentials,
        error,
        success,
        certificate_pem,
        private_key_pem,
        csrf_token,
    ))
}

fn render_certificate_page_with_status(
    status: StatusCode,
    account: NipAccount,
    user_email: String,
    environment: KSeFEnvironment,
    can_manage_credentials: bool,
    error: Option<String>,
    success: Option<String>,
    certificate_pem: String,
    private_key_pem: String,
    csrf_token: String,
) -> Response {
    render_with_status(
        status,
        build_certificate_template(
            account,
            user_email,
            environment,
            can_manage_credentials,
            error,
            success,
            certificate_pem,
            private_key_pem,
            csrf_token,
        ),
    )
}

fn normalize_pem_input(raw: &str) -> String {
    raw.replace("\r\n", "\n").trim().to_string()
}

fn validate_certificate_pair(cert_pem: &[u8], key_pem: &[u8]) -> Result<(), String> {
    let cert = X509::from_pem(cert_pem)
        .map_err(|e| format!("nie udało się odczytać certyfikatu PEM: {e}"))?;
    let key = PKey::private_key_from_pem(key_pem)
        .map_err(|e| format!("nie udało się odczytać klucza prywatnego PEM: {e}"))?;
    let cert_public_key = cert
        .public_key()
        .map_err(|e| format!("nie udało się odczytać klucza publicznego z certyfikatu: {e}"))?;

    if !cert_public_key.public_eq(&key) {
        return Err("certyfikat nie pasuje do podanego klucza prywatnego".to_string());
    }

    Ok(())
}

async fn parse_certificate_upload(
    mut multipart: Multipart,
    session: &Session,
) -> Result<CertificateUploadData, Response> {
    let expected_csrf = session
        .get::<String>(CSRF_SESSION_KEY)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session read error: {e}"),
            )
                .into_response()
        })?
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "Żądanie odrzucone: nieprawidłowy token CSRF",
            )
                .into_response()
        })?;

    let mut provided_csrf: Option<String> = None;
    let mut upload = CertificateUploadData::default();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("nie udało się odczytać danych formularza: {e}"),
        )
            .into_response()
    })? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "_csrf" => {
                provided_csrf = Some(field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("nie udało się odczytać tokenu CSRF: {e}"),
                    )
                        .into_response()
                })?);
            }
            "action" => {
                upload.action = field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("nie udało się odczytać akcji formularza: {e}"),
                    )
                        .into_response()
                })?;
            }
            "certificate_pem" => {
                upload.certificate_pem_text = field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("nie udało się odczytać pola certyfikatu: {e}"),
                    )
                        .into_response()
                })?;
            }
            "private_key_pem" => {
                upload.private_key_pem_text = field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("nie udało się odczytać pola klucza prywatnego: {e}"),
                    )
                        .into_response()
                })?;
            }
            "certificate_file" => {
                let bytes = field.bytes().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("nie udało się odczytać pliku certyfikatu: {e}"),
                    )
                        .into_response()
                })?;
                if !bytes.is_empty() {
                    upload.certificate_pem_file = Some(bytes.to_vec());
                }
            }
            "private_key_file" => {
                let bytes = field.bytes().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("nie udało się odczytać pliku klucza prywatnego: {e}"),
                    )
                        .into_response()
                })?;
                if !bytes.is_empty() {
                    upload.private_key_pem_file = Some(bytes.to_vec());
                }
            }
            _ => {}
        }
    }

    if provided_csrf.as_deref() != Some(expected_csrf.as_str()) {
        return Err((
            StatusCode::FORBIDDEN,
            "Żądanie odrzucone: nieprawidłowy token CSRF",
        )
            .into_response());
    }

    Ok(upload)
}

fn resolve_uploaded_pem(
    uploaded: Option<Vec<u8>>,
    pasted: &str,
    label: &str,
) -> Result<String, String> {
    match uploaded {
        Some(bytes) => String::from_utf8(bytes)
            .map(|raw| normalize_pem_input(&raw))
            .map_err(|e| format!("{label}: plik nie jest poprawnym tekstem UTF-8 PEM: {e}")),
        None => Ok(normalize_pem_input(pasted)),
    }
}

// --- Form data ---

#[derive(Deserialize)]
pub struct AddAccountFormData {
    pub nip: String,
    pub display_name: String,
}

#[derive(Default)]
struct CertificateUploadData {
    action: String,
    certificate_pem_text: String,
    private_key_pem_text: String,
    certificate_pem_file: Option<Vec<u8>>,
    private_key_pem_file: Option<Vec<u8>>,
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
    match state.nip_account_repo.find_by_nip(&nip).await {
        Ok(Some(existing)) => {
            let already_linked = match state.nip_account_repo.verify_access(&auth.id, &nip).await {
                Ok(Some(_)) => true,
                Ok(None) => false,
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

            if !already_linked {
                return render_with_status(
                    StatusCode::FORBIDDEN,
                    AccountAddTemplate {
                        active: "/accounts",
                        nip_prefix: None,
                        user_email: auth.email,
                        error: Some(format!(
                            "NIP {} jest już zapisany w aplikacji. Certyfikat i dostęp są prowadzone per NIP, więc istniejącego konta nie da się samodzielnie dopiąć do innego użytkownika.",
                            existing.nip
                        )),
                        nip: form_nip,
                        display_name: form_display_name,
                        csrf_token,
                    },
                );
            }

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
            return Redirect::to("/accounts").into_response();
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
            if let Err(e) = state
                .nip_account_repo
                .grant_access(&auth.id, &account.id, true)
                .await
            {
                return render_with_status(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AccountAddTemplate {
                        active: "/accounts",
                        nip_prefix: None,
                        user_email: auth.email,
                        error: Some(format!(
                            "Konto NIP utworzono, ale nie udało się przypisać właściciela konta: {e}"
                        )),
                        nip: form_nip,
                        display_name: form_display_name,
                        csrf_token,
                    },
                );
            }

            Redirect::to("/accounts").into_response()
        }
        Err(e) => render_with_status(
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
        ),
    }
}

pub async fn certificate_form(
    State(state): State<AppState>,
    nip_ctx: crate::extractors::NipContext,
    session: Session,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let can_manage_credentials = match state
        .nip_account_repo
        .can_manage_credentials(&nip_ctx.user.id, &nip_ctx.account.id)
        .await
    {
        Ok(allowed) => allowed,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udało się sprawdzić uprawnień do certyfikatu: {e}"),
            )
                .into_response();
        }
    };
    render_certificate_page(
        nip_ctx.account,
        nip_ctx.user.email,
        state.ksef_environment,
        can_manage_credentials,
        None,
        None,
        String::new(),
        String::new(),
        csrf_token,
    )
}

pub async fn certificate_save(
    State(state): State<AppState>,
    nip_ctx: crate::extractors::NipContext,
    headers: HeaderMap,
    session: Session,
    multipart: Multipart,
) -> Response {
    let upload = match parse_certificate_upload(multipart, &session).await {
        Ok(upload) => upload,
        Err(response) => return response,
    };
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let user_email = nip_ctx.user.email;
    let user_id = nip_ctx.user.id;
    let mut account = nip_ctx.account;
    let can_manage_credentials = match state
        .nip_account_repo
        .can_manage_credentials(&user_id, &account.id)
        .await
    {
        Ok(allowed) => allowed,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udało się sprawdzić uprawnień do certyfikatu: {e}"),
            )
                .into_response();
        }
    };
    let audit_ip = client_ip(&headers);

    if !can_manage_credentials {
        return render_certificate_page_with_status(
            StatusCode::FORBIDDEN,
            account,
            user_email,
            state.ksef_environment,
            false,
            Some(
                "Tylko właściciel dostępu do tego NIP może zapisywać lub usuwać certyfikat."
                    .to_string(),
            ),
            None,
            String::new(),
            String::new(),
            csrf_token,
        );
    }

    if upload.action == "clear" {
        account.cert_pem = None;
        account.key_pem = None;
        account.cert_auto_generated = false;
        account.updated_at = Utc::now();

        return match state.nip_account_repo.update_credentials(&account).await {
            Ok(()) => {
                audit_log::log_action(
                    &state,
                    &user_id,
                    &user_email,
                    Some(&account.nip),
                    AuditAction::DeleteCertificate,
                    Some("removed stored certificate pair".to_string()),
                    audit_ip.clone(),
                )
                .await;
                render_certificate_page(
                    account,
                    user_email,
                    state.ksef_environment,
                    true,
                    None,
                    Some("Usunięto zapisany certyfikat i klucz z konta.".to_string()),
                    String::new(),
                    String::new(),
                    csrf_token,
                )
            }
            Err(e) => render_certificate_page(
                account,
                user_email,
                state.ksef_environment,
                true,
                Some(format!(
                    "Nie udało się usunąć zapisanych danych certyfikatu: {e}"
                )),
                None,
                String::new(),
                String::new(),
                csrf_token,
            ),
        };
    }

    let certificate_pem = match resolve_uploaded_pem(
        upload.certificate_pem_file,
        &upload.certificate_pem_text,
        "certyfikat",
    ) {
        Ok(value) => value,
        Err(err) => {
            return render_certificate_page(
                account,
                user_email,
                state.ksef_environment,
                true,
                Some(err),
                None,
                upload.certificate_pem_text,
                upload.private_key_pem_text,
                csrf_token,
            );
        }
    };
    let private_key_pem = match resolve_uploaded_pem(
        upload.private_key_pem_file,
        &upload.private_key_pem_text,
        "klucz prywatny",
    ) {
        Ok(value) => value,
        Err(err) => {
            return render_certificate_page(
                account,
                user_email,
                state.ksef_environment,
                true,
                Some(err),
                None,
                certificate_pem,
                upload.private_key_pem_text,
                csrf_token,
            );
        }
    };

    if certificate_pem.is_empty() || private_key_pem.is_empty() {
        return render_certificate_page(
            account,
            user_email,
            state.ksef_environment,
            true,
            Some("Podaj jednocześnie certyfikat PEM i klucz prywatny PEM.".to_string()),
            None,
            certificate_pem,
            private_key_pem,
            csrf_token,
        );
    }

    if let Err(err) =
        validate_certificate_pair(certificate_pem.as_bytes(), private_key_pem.as_bytes())
    {
        return render_certificate_page(
            account,
            user_email,
            state.ksef_environment,
            true,
            Some(err),
            None,
            certificate_pem,
            private_key_pem,
            csrf_token,
        );
    }

    account.cert_pem = Some(certificate_pem.into_bytes());
    account.key_pem = Some(private_key_pem.into_bytes());
    account.cert_auto_generated = false;
    account.updated_at = Utc::now();

    match state.nip_account_repo.update_credentials(&account).await {
        Ok(()) => {
            audit_log::log_action(
                &state,
                &user_id,
                &user_email,
                Some(&account.nip),
                AuditAction::SaveCertificate,
                Some("saved stored certificate pair".to_string()),
                audit_ip,
            )
            .await;
            render_certificate_page(
                account,
                user_email,
                state.ksef_environment,
                true,
                None,
                Some(
                    "Zapisano certyfikat i klucz. Przy następnym uwierzytelnieniu XAdES aplikacja użyje tej pary dla tego NIP."
                        .to_string(),
                ),
                String::new(),
                String::new(),
                csrf_token,
            )
        }
        Err(e) => render_certificate_page(
            account,
            user_email,
            state.ksef_environment,
            true,
            Some(format!("Nie udało się zapisać certyfikatu: {e}")),
            None,
            String::new(),
            String::new(),
            csrf_token,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AddAccountFormData, add, certificate_form, certificate_save, normalize_pem_input,
        resolve_uploaded_pem, validate_certificate_pair,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, to_bytes};
    use axum::extract::{FromRequest, Multipart, State};
    use axum::http::{HeaderMap, Request, StatusCode};
    use chrono::Utc;
    use ksef_core::domain::account_scope::AccountScope;
    use ksef_core::domain::environment::KSeFEnvironment;
    use ksef_core::domain::nip::Nip;
    use ksef_core::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
    use ksef_core::domain::user::{User, UserId};
    use ksef_core::infra::batch::zip_builder::BatchFileBuilder;
    use ksef_core::infra::crypto::{AesCbcEncryptor, OpenSslSignerFactory, OpenSslXadesSigner};
    use ksef_core::infra::fa3::Fa3XmlConverter;
    use ksef_core::infra::http::rate_limiter::TokenBucketRateLimiter;
    use ksef_core::infra::http::retry::RetryPolicy;
    use ksef_core::infra::ksef::KSeFApiClient;
    use ksef_core::infra::qr::generator::QRCodeGenerator;
    use ksef_core::infra::whitelist::WhiteListClient;
    use ksef_core::services::audit_service::AuditService;
    use ksef_core::services::batch_service::BatchService;
    use ksef_core::services::company_lookup_service::CompanyLookupService;
    use ksef_core::services::export_service::ExportService;
    use ksef_core::services::fetch_service::FetchService;
    use ksef_core::services::invoice_service::InvoiceService;
    use ksef_core::services::offline_service::{OfflineConfig, OfflineService};
    use ksef_core::services::permission_service::PermissionService;
    use ksef_core::services::qr_service::QRService;
    use ksef_core::services::session_service::{AuthMethod, SessionService};
    use ksef_core::services::token_mgmt_service::TokenMgmtService;
    use openssl::asn1::Asn1Time;
    use openssl::bn::BigNum;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::X509;
    use tower_sessions::{MemoryStore, Session};

    use crate::auth_rate_limit::AuthRateLimiter;
    use crate::csrf::CSRF_SESSION_KEY;
    use crate::extractors::{AuthUser, CsrfForm, NipContext};
    use crate::state::AppState;

    fn make_user(email: &str) -> User {
        let now = Utc::now();
        User {
            id: UserId::new(),
            email: email.to_string(),
            password_hash: "test-password-hash".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn make_account(nip: &Nip) -> NipAccount {
        let now = Utc::now();
        NipAccount {
            id: NipAccountId::new(),
            nip: nip.clone(),
            display_name: format!("Firma {nip}"),
            ksef_auth_method: KSeFAuthMethod::Xades,
            ksef_auth_token: None,
            cert_pem: None,
            key_pem: None,
            cert_auto_generated: false,
            created_at: now,
            updated_at: now,
        }
    }

    async fn test_state(
        environment: KSeFEnvironment,
    ) -> (AppState, sqlx::SqlitePool, std::path::PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "ksef-server-accounts-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let database_url = format!("sqlite://{}", db_path.display());
        let db = crate::db_backend::connect(
            &database_url,
            Arc::new(ksef_core::infra::crypto::CertificateSecretBox::insecure_dev()),
        )
        .await
        .unwrap();
        let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();

        let ksef = Arc::new(KSeFApiClient::with_http_controls(
            environment,
            Arc::new(TokenBucketRateLimiter::default()),
            RetryPolicy::default(),
        ));
        let fallback_nip = Nip::parse("5260250274").unwrap();
        let fallback_signer =
            Arc::new(OpenSslXadesSigner::generate_self_signed_for_nip(&fallback_nip).unwrap());
        let signer_factory = Arc::new(OpenSslSignerFactory);
        let session_service = Arc::new(SessionService::with_signer_factory(
            ksef.clone(),
            fallback_signer,
            signer_factory,
            db.nip_account_repo.clone(),
            ksef.clone(),
            db.session_repo.clone(),
            environment,
            AuthMethod::Xades,
        ));
        let invoice_service = Arc::new(InvoiceService::with_atomic(
            db.invoice_repo.clone(),
            db.job_queue.clone(),
            db.atomic_scope_factory.clone(),
        ));
        let fetch_service = Arc::new(FetchService::new(
            session_service.clone(),
            ksef.clone(),
            db.invoice_repo.clone(),
            Arc::new(Fa3XmlConverter),
        ));
        let company_lookup_service = Arc::new(CompanyLookupService::new(
            db.company_cache.clone(),
            Arc::new(WhiteListClient::new()),
        ));
        let permission_service = Arc::new(PermissionService::new(ksef.clone()));
        let token_mgmt_service = Arc::new(TokenMgmtService::new(ksef.clone()));
        let export_service = Arc::new(ExportService::new(ksef.clone(), Arc::new(AesCbcEncryptor)));
        let audit_service = Arc::new(AuditService::new(db.audit_log_repo.clone()));
        let batch_service = Arc::new(BatchService::new(
            ksef.clone(),
            Arc::new(BatchFileBuilder::default()),
        ));
        let qr_renderer = Arc::new(QRCodeGenerator);
        let qr_service = Arc::new(QRService::new(environment, qr_renderer.clone()));
        let offline_service = Arc::new(OfflineService::new(
            QRService::new(environment, qr_renderer),
            OfflineConfig::default(),
        ));

        let state = AppState {
            ksef_environment: environment,
            user_repo: db.user_repo.clone(),
            nip_account_repo: db.nip_account_repo.clone(),
            company_lookup_service,
            invoice_sequence: db.invoice_sequence.clone(),
            invoice_service,
            fetch_service,
            session_service,
            batch_service,
            permission_service,
            token_mgmt_service,
            local_token_repo: db.local_token_repo.clone(),
            export_service,
            offline_service,
            qr_service,
            audit_service,
            export_keys: Arc::new(Mutex::new(HashMap::new())),
            fetch_jobs: Arc::new(Mutex::new(HashMap::new())),
            auth_rate_limiter: AuthRateLimiter::default(),
            allowed_emails: Vec::new(),
        };

        (state, pool, db_path)
    }

    fn auth_user(user: &User) -> AuthUser {
        AuthUser {
            id: user.id.clone(),
            email: user.email.clone(),
        }
    }

    async fn session_with_csrf(token: &str) -> Session {
        let session = Session::new(None, Arc::new(MemoryStore::default()), None);
        session
            .insert(CSRF_SESSION_KEY, token.to_string())
            .await
            .unwrap();
        session
    }

    async fn response_text(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn nip_ctx(user: &User, account: NipAccount, scope: AccountScope) -> NipContext {
        NipContext {
            user: auth_user(user),
            account,
            scope,
        }
    }

    async fn multipart_from_text_fields(
        state: &AppState,
        csrf: &str,
        certificate_pem: &str,
        private_key_pem: &str,
    ) -> Multipart {
        let boundary = "X-BOUNDARY";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n\
--{boundary}\r\nContent-Disposition: form-data; name=\"certificate_pem\"\r\n\r\n{certificate_pem}\r\n\
--{boundary}\r\nContent-Disposition: form-data; name=\"private_key_pem\"\r\n\r\n{private_key_pem}\r\n\
--{boundary}--\r\n"
        );
        let request = Request::builder()
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();

        Multipart::from_request(request, state).await.unwrap()
    }

    fn generate_pair() -> (Vec<u8>, Vec<u8>) {
        let rsa = Rsa::generate(2048).unwrap();
        let pkey = PKey::from_rsa(rsa).unwrap();

        let mut name = openssl::x509::X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "test-cert").unwrap();
        let name = name.build();

        let mut builder = X509::builder().unwrap();
        builder.set_version(2).unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(&name).unwrap();
        builder.set_pubkey(&pkey).unwrap();
        let serial = BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap();
        builder.set_serial_number(&serial).unwrap();
        let not_before = Asn1Time::days_from_now(0).unwrap();
        let not_after = Asn1Time::days_from_now(30).unwrap();
        builder.set_not_before(&not_before).unwrap();
        builder.set_not_after(&not_after).unwrap();
        builder.sign(&pkey, MessageDigest::sha256()).unwrap();

        let cert_pem = builder.build().to_pem().unwrap();
        let key_pem = pkey.private_key_to_pem_pkcs8().unwrap();
        (cert_pem, key_pem)
    }

    #[test]
    fn normalize_pem_input_trims_and_normalizes_newlines() {
        let normalized = normalize_pem_input("  line1\r\nline2\r\n");
        assert_eq!(normalized, "line1\nline2");
    }

    #[test]
    fn validate_certificate_pair_accepts_matching_pair() {
        let (cert_pem, key_pem) = generate_pair();
        assert!(validate_certificate_pair(&cert_pem, &key_pem).is_ok());
    }

    #[test]
    fn validate_certificate_pair_rejects_mismatched_pair() {
        let (cert_pem, _) = generate_pair();
        let (_, other_key_pem) = generate_pair();
        let err = validate_certificate_pair(&cert_pem, &other_key_pem).unwrap_err();
        assert!(err.contains("nie pasuje"));
    }

    #[test]
    fn resolve_uploaded_pem_prefers_uploaded_file_contents() {
        let resolved =
            resolve_uploaded_pem(Some(b" file\r\npem ".to_vec()), "ignored", "certyfikat").unwrap();
        assert_eq!(resolved, "file\npem");
    }

    #[test]
    fn resolve_uploaded_pem_uses_text_fallback_without_file() {
        let resolved = resolve_uploaded_pem(None, " fallback\r\npem ", "certyfikat").unwrap();
        assert_eq!(resolved, "fallback\npem");
    }

    #[tokio::test]
    async fn add_rejects_existing_nip_for_other_user() {
        let (state, _pool, db_path) = test_state(KSeFEnvironment::Production).await;
        let owner = make_user("owner@example.com");
        let owner_id = state.user_repo.create(&owner).await.unwrap();
        let other = make_user("other@example.com");
        state.user_repo.create(&other).await.unwrap();

        let nip = Nip::parse("5260250274").unwrap();
        let account = make_account(&nip);
        state.nip_account_repo.create(&account).await.unwrap();
        state
            .nip_account_repo
            .grant_access(&owner_id, &account.id, true)
            .await
            .unwrap();

        let response = add(
            State(state.clone()),
            auth_user(&other),
            session_with_csrf("csrf-add").await,
            CsrfForm(AddAccountFormData {
                nip: nip.to_string(),
                display_name: "Inna Firma".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_text(response).await;
        assert!(body.contains("nie da się samodzielnie dopiąć"));
        assert!(
            state
                .nip_account_repo
                .verify_access(&other.id, &nip)
                .await
                .unwrap()
                .is_none()
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn certificate_save_persists_encrypted_pair_for_owner() {
        let (state, pool, db_path) = test_state(KSeFEnvironment::Production).await;
        let owner = make_user("owner@example.com");
        let owner_id = state.user_repo.create(&owner).await.unwrap();
        let nip = Nip::parse("5260250274").unwrap();
        let account = make_account(&nip);
        state.nip_account_repo.create(&account).await.unwrap();
        state
            .nip_account_repo
            .grant_access(&owner_id, &account.id, true)
            .await
            .unwrap();
        let (account, scope) = state
            .nip_account_repo
            .verify_access(&owner.id, &nip)
            .await
            .unwrap()
            .unwrap();
        let account_id = account.id.clone();
        let (cert_pem, key_pem) = generate_pair();
        let cert_text = String::from_utf8(cert_pem.clone()).unwrap();
        let key_text = String::from_utf8(key_pem.clone()).unwrap();

        let response = certificate_save(
            State(state.clone()),
            nip_ctx(&owner, account, scope),
            HeaderMap::new(),
            session_with_csrf("csrf-save").await,
            multipart_from_text_fields(&state, "csrf-save", &cert_text, &key_text).await,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("Zapisano certyfikat i klucz"));

        let stored = state
            .nip_account_repo
            .find_by_nip(&nip)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.cert_pem.as_deref(),
            Some(normalize_pem_input(&cert_text).as_bytes())
        );
        assert_eq!(
            stored.key_pem.as_deref(),
            Some(normalize_pem_input(&key_text).as_bytes())
        );

        let raw_cert: Option<String> =
            sqlx::query_scalar("SELECT cert_pem FROM nip_accounts WHERE id = ?1")
                .bind(account_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        let raw_key: Option<String> =
            sqlx::query_scalar("SELECT key_pem FROM nip_accounts WHERE id = ?1")
                .bind(account_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(raw_cert.as_deref().unwrap().starts_with("enc:v1:"));
        assert!(raw_key.as_deref().unwrap().starts_with("enc:v1:"));
        assert!(!raw_cert.as_deref().unwrap().contains("BEGIN CERTIFICATE"));
        assert!(!raw_key.as_deref().unwrap().contains("BEGIN PRIVATE KEY"));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn certificate_save_rejects_read_only_user() {
        let (state, _pool, db_path) = test_state(KSeFEnvironment::Production).await;
        let owner = make_user("owner@example.com");
        let owner_id = state.user_repo.create(&owner).await.unwrap();
        let operator = make_user("operator@example.com");
        let operator_id = state.user_repo.create(&operator).await.unwrap();
        let nip = Nip::parse("5260250274").unwrap();
        let account = make_account(&nip);
        state.nip_account_repo.create(&account).await.unwrap();
        state
            .nip_account_repo
            .grant_access(&owner_id, &account.id, true)
            .await
            .unwrap();
        state
            .nip_account_repo
            .grant_access(&operator_id, &account.id, false)
            .await
            .unwrap();
        let (account, scope) = state
            .nip_account_repo
            .verify_access(&operator.id, &nip)
            .await
            .unwrap()
            .unwrap();
        let (cert_pem, key_pem) = generate_pair();

        let response = certificate_save(
            State(state.clone()),
            nip_ctx(&operator, account, scope),
            HeaderMap::new(),
            session_with_csrf("csrf-save").await,
            multipart_from_text_fields(
                &state,
                "csrf-save",
                &String::from_utf8(cert_pem).unwrap(),
                &String::from_utf8(key_pem).unwrap(),
            )
            .await,
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_text(response).await;
        assert!(body.contains("Tylko właściciel dostępu do tego NIP"));

        let stored = state
            .nip_account_repo
            .find_by_nip(&nip)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.cert_pem.is_none());
        assert!(stored.key_pem.is_none());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn certificate_form_shows_read_only_notice_for_non_manager() {
        let (state, _pool, db_path) = test_state(KSeFEnvironment::Production).await;
        let owner = make_user("owner@example.com");
        let owner_id = state.user_repo.create(&owner).await.unwrap();
        let operator = make_user("operator@example.com");
        let operator_id = state.user_repo.create(&operator).await.unwrap();
        let nip = Nip::parse("5260250274").unwrap();
        let account = make_account(&nip);
        state.nip_account_repo.create(&account).await.unwrap();
        state
            .nip_account_repo
            .grant_access(&owner_id, &account.id, true)
            .await
            .unwrap();
        state
            .nip_account_repo
            .grant_access(&operator_id, &account.id, false)
            .await
            .unwrap();
        let (account, scope) = state
            .nip_account_repo
            .verify_access(&operator.id, &nip)
            .await
            .unwrap()
            .unwrap();

        let response = certificate_form(
            State(state),
            nip_ctx(&operator, account, scope),
            session_with_csrf("csrf-form").await,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("To konto jest tylko do odczytu"));
        assert!(body.contains("Brak uprawnienia do zarządzania certyfikatem"));

        let _ = std::fs::remove_file(db_path);
    }
}
