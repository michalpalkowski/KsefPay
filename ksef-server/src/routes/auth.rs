use askama::Template;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;
use tower_sessions::Session;

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::user::{User, UserId};
use ksef_core::domain::workspace::WorkspaceInvite;
use ksef_core::error::RepositoryError;

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::{CURRENT_WORKSPACE_SESSION_KEY, CsrfForm};
use crate::request_meta::client_ip;
use crate::state::AppState;
use crate::workspace_invites::{
    InviteResolutionError, require_invite_email, resolve_pending_invite,
};

#[derive(Template)]
#[template(path = "pages/login.html")]
struct LoginTemplate {
    error: Option<String>,
    email: String,
    csrf_token: String,
    invite_token: String,
    invite_message: Option<String>,
    register_href: String,
}

#[derive(Template)]
#[template(path = "pages/register.html")]
struct RegisterTemplate {
    error: Option<String>,
    email: String,
    csrf_token: String,
    invite_token: String,
    invite_message: Option<String>,
    login_href: String,
}

#[derive(Deserialize)]
pub struct LoginFormData {
    pub email: String,
    pub password: String,
    pub invite_token: Option<String>,
}

#[derive(Deserialize)]
pub struct RegisterFormData {
    pub email: String,
    pub password: String,
    pub password_confirm: String,
    pub invite_token: Option<String>,
}

#[derive(Deserialize)]
pub struct InviteQuery {
    pub invite: Option<String>,
}

#[derive(Debug, Error)]
enum RegistrationGateError {
    #[error("registration is closed")]
    Closed,
    #[error(transparent)]
    Invite(#[from] InviteResolutionError),
}

enum RegistrationAuthorization {
    BootstrapAdmin,
    Invite(WorkspaceInvite),
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

fn with_retry_after(mut response: Response, retry_after_seconds: u64) -> Response {
    let header_value = HeaderValue::from_str(&retry_after_seconds.to_string())
        .unwrap_or_else(|_| HeaderValue::from_static("60"));
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, header_value);
    response
}

fn auth_rate_limit_retry_after(state: &AppState, headers: &HeaderMap) -> Option<u64> {
    let key = client_ip(headers).unwrap_or_else(|| "unknown".to_string());
    state.auth_rate_limiter.check(&key)
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn normalize_invite_token(token: Option<String>) -> String {
    token.unwrap_or_default().trim().to_string()
}

fn register_href(invite_token: &str) -> String {
    if invite_token.is_empty() {
        "/register".to_string()
    } else {
        format!("/register?invite={invite_token}")
    }
}

fn login_href(invite_token: &str) -> String {
    if invite_token.is_empty() {
        "/login".to_string()
    } else {
        format!("/login?invite={invite_token}")
    }
}

fn render_login_template(
    status: StatusCode,
    error: Option<String>,
    email: String,
    csrf_token: String,
    invite_token: String,
    invite_message: Option<String>,
) -> Response {
    render_with_status(
        status,
        LoginTemplate {
            error,
            email,
            csrf_token,
            register_href: register_href(&invite_token),
            invite_token,
            invite_message,
        },
    )
}

fn render_register_template(
    status: StatusCode,
    error: Option<String>,
    email: String,
    csrf_token: String,
    invite_token: String,
    invite_message: Option<String>,
) -> Response {
    render_with_status(
        status,
        RegisterTemplate {
            error,
            email,
            csrf_token,
            login_href: login_href(&invite_token),
            invite_token,
            invite_message,
        },
    )
}

async fn invite_message(state: &AppState, invite: &WorkspaceInvite) -> String {
    match state.workspace_repo.find_by_id(&invite.workspace_id).await {
        Ok(workspace) => format!(
            "Zaproszenie do workspace {} jako {} dla {}.",
            workspace.display_name,
            invite.role.display_name(),
            invite.email
        ),
        Err(_) => format!(
            "Zaproszenie do workspace {} jako {} dla {}.",
            invite.workspace_id,
            invite.role.display_name(),
            invite.email
        ),
    }
}

async fn invite_page_context(
    state: &AppState,
    invite_token: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    if invite_token.is_empty() {
        return (None, None, None);
    }

    match resolve_pending_invite(state, invite_token).await {
        Ok(invite) => (
            None,
            Some(invite.email.clone()),
            Some(invite_message(state, &invite).await),
        ),
        Err(err) => (
            Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
            None,
            None,
        ),
    }
}

async fn resolve_registration_authorization(
    state: &AppState,
    email: &str,
    invite_token: &str,
) -> Result<RegistrationAuthorization, RegistrationGateError> {
    if state.allowed_emails.contains(&email.to_lowercase()) {
        return Ok(RegistrationAuthorization::BootstrapAdmin);
    }

    if invite_token.is_empty() {
        return Err(RegistrationGateError::Closed);
    }

    let invite = resolve_pending_invite(state, invite_token).await?;
    require_invite_email(&invite, email)?;
    Ok(RegistrationAuthorization::Invite(invite))
}

async fn apply_invite_membership(
    state: &AppState,
    session: &Session,
    user: &User,
    invite: &WorkspaceInvite,
) -> Result<(), RepositoryError> {
    state
        .workspace_repo
        .add_member(&invite.workspace_id, &user.id, invite.role)
        .await?;
    state.workspace_repo.accept_invite(&invite.id).await?;
    session
        .insert(CURRENT_WORKSPACE_SESSION_KEY, invite.workspace_id.to_string())
        .await
        .map_err(|e| RepositoryError::Storage(format!("session write error: {e}")))?;
    Ok(())
}

fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let (local, domain) = (parts[0], parts[1]);
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') || domain.contains("..") {
        return false;
    }
    let last_part = domain.rsplit('.').next().unwrap_or("");
    last_part.len() >= 2 && last_part.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn validate_password_strength(password: &str) -> Result<(), String> {
    if password.len() < 8 {
        return Err("Hasło musi mieć co najmniej 8 znaków".to_string());
    }
    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        return Err("Hasło musi zawierać co najmniej jedną dużą literę".to_string());
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err("Hasło musi zawierać co najmniej jedną cyfrę".to_string());
    }
    if !password.chars().any(|c| !c.is_ascii_alphanumeric()) {
        return Err("Hasło musi zawierać co najmniej jeden znak specjalny (np. !@#$%)".to_string());
    }
    Ok(())
}

pub async fn login_page(
    State(state): State<AppState>,
    session: Session,
    Query(query): Query<InviteQuery>,
) -> Response {
    if let Ok(Some(_)) = session.get::<String>("user_id").await {
        return Redirect::to("/accounts").into_response();
    }

    let invite_token = normalize_invite_token(query.invite);
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let (error, invited_email, invited_message) = invite_page_context(&state, &invite_token).await;

    render(LoginTemplate {
        error,
        email: invited_email.unwrap_or_default(),
        csrf_token,
        invite_token: invite_token.clone(),
        invite_message: invited_message,
        register_href: register_href(&invite_token),
    })
}

pub async fn login(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    CsrfForm(form): CsrfForm<LoginFormData>,
) -> Response {
    let email = normalize_email(&form.email);
    let invite_token = normalize_invite_token(form.invite_token);
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    if let Some(retry_after) = auth_rate_limit_retry_after(&state, &headers) {
        return with_retry_after(
            render_login_template(
                StatusCode::TOO_MANY_REQUESTS,
                Some(format!(
                    "Zbyt wiele prób logowania. Spróbuj ponownie za {retry_after} sekund."
                )),
                email,
                csrf_token,
                invite_token,
                None,
            ),
            retry_after,
        );
    }

    if email.is_empty() || form.password.is_empty() {
        return render_login_template(
            StatusCode::BAD_REQUEST,
            Some("Email i hasło są wymagane".to_string()),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    let user = match state.user_repo.find_by_email(&email).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return render_login_template(
                StatusCode::UNAUTHORIZED,
                Some("Nieprawidłowy email lub hasło".to_string()),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
        Err(e) => {
            return render_login_template(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(format!("Błąd serwera: {e}")),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
    };

    let Ok(parsed_hash) = PasswordHash::new(&user.password_hash) else {
        return render_login_template(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("Błąd weryfikacji hasła".to_string()),
            email,
            csrf_token,
            invite_token,
            None,
        );
    };

    if Argon2::default()
        .verify_password(form.password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return render_login_template(
            StatusCode::UNAUTHORIZED,
            Some("Nieprawidłowy email lub hasło".to_string()),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    if let Err(e) = session.insert("user_id", user.id.to_string()).await {
        return render_login_template(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(format!("Błąd sesji: {e}")),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    if invite_token.is_empty() {
        let workspace = match state
            .workspace_repo
            .ensure_default_workspace(&user.id, &user.email)
            .await
        {
            Ok(workspace) => workspace,
            Err(e) => {
                return render_login_template(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(format!("Nie udało się przygotować workspace: {e}")),
                    email,
                    csrf_token,
                    invite_token,
                    None,
                );
            }
        };
        if let Err(e) = session
            .insert(
                CURRENT_WORKSPACE_SESSION_KEY,
                workspace.workspace.id.to_string(),
            )
            .await
        {
            return render_login_template(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(format!("Błąd sesji workspace: {e}")),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
    } else {
        let invite = match resolve_pending_invite(&state, &invite_token).await {
            Ok(invite) => invite,
            Err(err) => {
                return render_login_template(
                    StatusCode::FORBIDDEN,
                    Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                    email,
                    csrf_token,
                    invite_token,
                    None,
                );
            }
        };
        if let Err(err) = require_invite_email(&invite, &user.email) {
            return render_login_template(
                StatusCode::FORBIDDEN,
                Some(format!("Zaproszenie nie pasuje do tego konta: {err}")),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
        if let Err(err) = apply_invite_membership(&state, &session, &user, &invite).await {
            return render_login_template(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(format!("Nie udało się aktywować zaproszenia: {err}")),
                email,
                csrf_token,
                invite_token.clone(),
                Some(invite_message(&state, &invite).await),
            );
        }
    }

    log_audit_action(
        &state,
        &user.id,
        &user.email,
        None,
        AuditAction::Login,
        None,
        client_ip(&headers),
    )
    .await;

    Redirect::to("/accounts").into_response()
}

pub async fn register_page(
    State(state): State<AppState>,
    session: Session,
    Query(query): Query<InviteQuery>,
) -> Response {
    if let Ok(Some(_)) = session.get::<String>("user_id").await {
        return Redirect::to("/accounts").into_response();
    }

    let invite_token = normalize_invite_token(query.invite);
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let (error, invited_email, invited_message) = invite_page_context(&state, &invite_token).await;

    render(RegisterTemplate {
        error,
        email: invited_email.unwrap_or_default(),
        csrf_token,
        invite_token: invite_token.clone(),
        invite_message: invited_message,
        login_href: login_href(&invite_token),
    })
}

pub async fn register(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    CsrfForm(form): CsrfForm<RegisterFormData>,
) -> Response {
    let email = normalize_email(&form.email);
    let invite_token = normalize_invite_token(form.invite_token);
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    if let Some(retry_after) = auth_rate_limit_retry_after(&state, &headers) {
        return with_retry_after(
            render_register_template(
                StatusCode::TOO_MANY_REQUESTS,
                Some(format!(
                    "Zbyt wiele prób rejestracji. Spróbuj ponownie za {retry_after} sekund."
                )),
                email,
                csrf_token,
                invite_token,
                None,
            ),
            retry_after,
        );
    }

    if email.is_empty() || form.password.is_empty() {
        return render_register_template(
            StatusCode::BAD_REQUEST,
            Some("Email i hasło są wymagane".to_string()),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    if !is_valid_email(&email) {
        return render_register_template(
            StatusCode::BAD_REQUEST,
            Some("Nieprawidłowy adres email".to_string()),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    let authorization =
        match resolve_registration_authorization(&state, &email, &invite_token).await {
            Ok(authorization) => authorization,
            Err(RegistrationGateError::Closed) => {
                return render_register_template(
                    StatusCode::FORBIDDEN,
                    Some(
                        "Rejestracja jest zamknięta. Poproś administratora workspace o zaproszenie."
                            .to_string(),
                    ),
                    email,
                    csrf_token,
                    invite_token,
                    None,
                );
            }
            Err(RegistrationGateError::Invite(err)) => {
                return render_register_template(
                    StatusCode::FORBIDDEN,
                    Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                    email,
                    csrf_token,
                    invite_token,
                    None,
                );
            }
        };

    if let Err(msg) = validate_password_strength(&form.password) {
        return render_register_template(
            StatusCode::BAD_REQUEST,
            Some(msg),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    if form.password != form.password_confirm {
        return render_register_template(
            StatusCode::BAD_REQUEST,
            Some("Hasła nie są zgodne".to_string()),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    match state.user_repo.find_by_email(&email).await {
        Ok(Some(_)) => {
            return render_register_template(
                StatusCode::CONFLICT,
                Some("Konto z tym adresem email już istnieje".to_string()),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
        Ok(None) => {}
        Err(e) => {
            return render_register_template(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(format!("Błąd serwera: {e}")),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
    }

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = match Argon2::default().hash_password(form.password.as_bytes(), &salt) {
        Ok(hash) => hash.to_string(),
        Err(e) => {
            return render_register_template(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(format!("Błąd hashowania hasła: {e}")),
                email,
                csrf_token,
                invite_token,
                None,
            );
        }
    };

    let now = Utc::now();
    let user = User {
        id: UserId::new(),
        email: email.clone(),
        password_hash,
        created_at: now,
        updated_at: now,
    };

    if let Err(e) = state.user_repo.create(&user).await {
        return render_register_template(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(format!("Nie udało się utworzyć konta: {e}")),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    if let Err(e) = session.insert("user_id", user.id.to_string()).await {
        return render_register_template(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(format!("Błąd sesji: {e}")),
            email,
            csrf_token,
            invite_token,
            None,
        );
    }

    match authorization {
        RegistrationAuthorization::BootstrapAdmin => {
            let workspace = match state
                .workspace_repo
                .ensure_default_workspace(&user.id, &user.email)
                .await
            {
                Ok(workspace) => workspace,
                Err(e) => {
                    return render_register_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!("Nie udało się przygotować workspace: {e}")),
                        email,
                        csrf_token,
                        invite_token,
                        None,
                    );
                }
            };
            if let Err(e) = session
                .insert(
                    CURRENT_WORKSPACE_SESSION_KEY,
                    workspace.workspace.id.to_string(),
                )
                .await
            {
                return render_register_template(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(format!("Błąd sesji workspace: {e}")),
                    email,
                    csrf_token,
                    invite_token,
                    None,
                );
            }
        }
        RegistrationAuthorization::Invite(invite) => {
            let invite_info = invite_message(&state, &invite).await;
            if let Err(e) = apply_invite_membership(&state, &session, &user, &invite).await {
                return render_register_template(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(format!("Nie udało się aktywować zaproszenia: {e}")),
                    email,
                    csrf_token,
                    invite_token,
                    Some(invite_info),
                );
            }
        }
    }

    log_audit_action(
        &state,
        &user.id,
        &user.email,
        None,
        AuditAction::Register,
        None,
        client_ip(&headers),
    )
    .await;

    Redirect::to("/accounts").into_response()
}

pub async fn logout(session: Session) -> Response {
    let _ = session.delete().await;
    Redirect::to("/login").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::body::to_bytes;
    use chrono::Duration;
    use tower_sessions::{MemoryStore, Session};

    use ksef_core::domain::environment::KSeFEnvironment;
    use ksef_core::domain::workspace::{WorkspaceInviteId, WorkspaceRole};
    use ksef_core::infra::batch::zip_builder::BatchFileBuilder;
    use ksef_core::infra::crypto::{
        AesCbcEncryptor, OpenSslSignerFactory, OpenSslXadesSigner,
    };
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

    use crate::auth_rate_limit::AuthRateLimiter;
    use crate::email::NoopEmailSender;

    async fn test_state(allowed_emails: Vec<String>) -> (AppState, std::path::PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "ksef-server-auth-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let database_url = format!("sqlite://{}", db_path.display());
        let db = crate::db_backend::connect(
            &database_url,
            Arc::new(ksef_core::infra::crypto::CertificateSecretBox::insecure_dev()),
        )
        .await
        .unwrap();

        let ksef = Arc::new(KSeFApiClient::with_http_controls(
            KSeFEnvironment::Production,
            Arc::new(TokenBucketRateLimiter::default()),
            RetryPolicy::default(),
        ));
        let fallback_nip = ksef_core::domain::nip::Nip::parse("5260250274").unwrap();
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
            KSeFEnvironment::Production,
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
        let qr_service = Arc::new(QRService::new(KSeFEnvironment::Production, qr_renderer.clone()));
        let offline_service = Arc::new(OfflineService::new(
            QRService::new(KSeFEnvironment::Production, qr_renderer),
            OfflineConfig::default(),
        ));

        let state = AppState {
            ksef_environment: KSeFEnvironment::Production,
            user_repo: db.user_repo.clone(),
            nip_account_repo: db.nip_account_repo.clone(),
            workspace_repo: db.workspace_repo.clone(),
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
            email_sender: Arc::new(NoopEmailSender),
            export_keys: Arc::new(Mutex::new(HashMap::new())),
            fetch_jobs: Arc::new(Mutex::new(HashMap::new())),
            auth_rate_limiter: AuthRateLimiter::default(),
            public_base_url: "https://app.example.test".to_string(),
            allowed_emails,
        };

        (state, db_path)
    }

    async fn session_with_csrf(token: &str) -> Session {
        let session = Session::new(None, Arc::new(MemoryStore::default()), None);
        session
            .insert(crate::csrf::CSRF_SESSION_KEY, token.to_string())
            .await
            .unwrap();
        session
    }

    async fn response_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn password_hash(password: &str) -> String {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    fn make_user(email: &str, password: &str) -> User {
        let now = Utc::now();
        User {
            id: UserId::new(),
            email: email.to_string(),
            password_hash: password_hash(password),
            created_at: now,
            updated_at: now,
        }
    }

    async fn create_invite_for_workspace(
        state: &AppState,
        owner: &User,
        invited_email: &str,
        role: WorkspaceRole,
        raw_token: &str,
    ) -> ksef_core::domain::workspace::Workspace {
        let workspace = state
            .workspace_repo
            .ensure_default_workspace(&owner.id, &owner.email)
            .await
            .unwrap()
            .workspace;
        state
            .workspace_repo
            .create_invite(&WorkspaceInvite {
                id: WorkspaceInviteId::new(),
                workspace_id: workspace.id.clone(),
                email: invited_email.to_string(),
                role,
                token_hash: crate::workspace_invites::hash_invite_token(raw_token),
                expires_at: Utc::now() + Duration::days(7),
                accepted_at: None,
                revoked_at: None,
                created_by_user_id: owner.id.clone(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        workspace
    }

    #[tokio::test]
    async fn register_requires_invite_when_email_is_not_bootstrap_admin() {
        let (state, db_path) = test_state(Vec::new()).await;

        let response = register(
            State(state),
            session_with_csrf("csrf").await,
            HeaderMap::new(),
            CsrfForm(RegisterFormData {
                email: "user@example.com".to_string(),
                password: "Passw0rd!".to_string(),
                password_confirm: "Passw0rd!".to_string(),
                invite_token: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_text(response).await;
        assert!(body.contains("Poproś administratora workspace o zaproszenie"));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn bootstrap_admin_registration_creates_workspace() {
        let (state, db_path) = test_state(vec!["admin@example.com".to_string()]).await;

        let response = register(
            State(state.clone()),
            session_with_csrf("csrf").await,
            HeaderMap::new(),
            CsrfForm(RegisterFormData {
                email: "admin@example.com".to_string(),
                password: "Passw0rd!".to_string(),
                password_confirm: "Passw0rd!".to_string(),
                invite_token: None,
            }),
        )
        .await;

        assert!(response.status().is_redirection());
        let user = state
            .user_repo
            .find_by_email("admin@example.com")
            .await
            .unwrap()
            .unwrap();
        let workspaces = state.workspace_repo.list_for_user(&user.id).await.unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].membership.role, WorkspaceRole::Owner);

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn register_with_valid_invite_creates_membership() {
        let (state, db_path) = test_state(Vec::new()).await;
        let owner = make_user("owner@example.com", "OwnerPass1!");
        state.user_repo.create(&owner).await.unwrap();
        let raw_token = "invite.register.token";
        let workspace = create_invite_for_workspace(
            &state,
            &owner,
            "new.user@example.com",
            WorkspaceRole::Operator,
            raw_token,
        )
        .await;

        let response = register(
            State(state.clone()),
            session_with_csrf("csrf").await,
            HeaderMap::new(),
            CsrfForm(RegisterFormData {
                email: "new.user@example.com".to_string(),
                password: "Passw0rd!".to_string(),
                password_confirm: "Passw0rd!".to_string(),
                invite_token: Some(raw_token.to_string()),
            }),
        )
        .await;

        assert!(response.status().is_redirection());
        let user = state
            .user_repo
            .find_by_email("new.user@example.com")
            .await
            .unwrap()
            .unwrap();
        let membership = state
            .workspace_repo
            .find_membership(&workspace.id, &user.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(membership.role, WorkspaceRole::Operator);
        assert_eq!(membership.status, ksef_core::domain::workspace::WorkspaceMembershipStatus::Active);

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn login_with_valid_invite_grants_existing_user_membership() {
        let (state, db_path) = test_state(Vec::new()).await;
        let owner = make_user("owner@example.com", "OwnerPass1!");
        state.user_repo.create(&owner).await.unwrap();
        let invited = make_user("existing.user@example.com", "Passw0rd!");
        state.user_repo.create(&invited).await.unwrap();
        let raw_token = "invite.login.token";
        let workspace = create_invite_for_workspace(
            &state,
            &owner,
            "existing.user@example.com",
            WorkspaceRole::Admin,
            raw_token,
        )
        .await;

        let response = login(
            State(state.clone()),
            session_with_csrf("csrf").await,
            HeaderMap::new(),
            CsrfForm(LoginFormData {
                email: "existing.user@example.com".to_string(),
                password: "Passw0rd!".to_string(),
                invite_token: Some(raw_token.to_string()),
            }),
        )
        .await;

        assert!(response.status().is_redirection());
        let membership = state
            .workspace_repo
            .find_membership(&workspace.id, &invited.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(membership.role, WorkspaceRole::Admin);

        let _ = std::fs::remove_file(db_path);
    }
}
