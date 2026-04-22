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

use ksef_core::domain::application_access::ApplicationAccessInvite;
use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::user::{User, UserId};
use ksef_core::domain::workspace::WorkspaceInvite;
use ksef_core::error::RepositoryError;

use crate::application_access_invites::{
    ApplicationAccessInviteResolutionError, require_application_access_invite_email,
    resolve_pending_application_access_invite,
};
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
    workspace_invite_token: String,
    app_access_invite_token: String,
    invite_message: Option<String>,
    page_copy: String,
    extra_info: Option<String>,
    register_href: String,
}

#[derive(Template)]
#[template(path = "pages/register.html")]
struct RegisterTemplate {
    error: Option<String>,
    email: String,
    csrf_token: String,
    workspace_invite_token: String,
    app_access_invite_token: String,
    invite_message: Option<String>,
    page_copy: String,
    extra_info: Option<String>,
    login_href: String,
}

#[derive(Deserialize)]
pub struct LoginFormData {
    pub email: String,
    pub password: String,
    pub workspace_invite_token: Option<String>,
    pub app_access_invite_token: Option<String>,
}

#[derive(Deserialize)]
pub struct RegisterFormData {
    pub email: String,
    pub password: String,
    pub password_confirm: String,
    pub workspace_invite_token: Option<String>,
    pub app_access_invite_token: Option<String>,
}

#[derive(Deserialize)]
pub struct InviteQuery {
    pub workspace_invite: Option<String>,
    pub app_access_invite: Option<String>,
}

#[derive(Debug, Error)]
enum RegistrationGateError {
    #[error("registration is closed")]
    Closed,
    #[error("conflicting invite kinds in one request")]
    ConflictingInviteKinds,
    #[error(transparent)]
    WorkspaceInvite(#[from] InviteResolutionError),
    #[error(transparent)]
    ApplicationAccessInvite(#[from] ApplicationAccessInviteResolutionError),
}

enum RegistrationAuthorization {
    BootstrapAdmin,
    WorkspaceInvite(WorkspaceInvite),
    ApplicationAccessInvite(ApplicationAccessInvite),
}

#[derive(Debug, Clone, Default)]
struct InviteTokens {
    workspace: String,
    application_access: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InviteKind {
    None,
    Workspace,
    ApplicationAccess,
}

impl InviteTokens {
    fn kind(&self) -> InviteKind {
        if !self.workspace.is_empty() {
            InviteKind::Workspace
        } else if !self.application_access.is_empty() {
            InviteKind::ApplicationAccess
        } else {
            InviteKind::None
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

fn normalize_invite_tokens(
    workspace: Option<String>,
    application_access: Option<String>,
) -> Result<InviteTokens, RegistrationGateError> {
    let tokens = InviteTokens {
        workspace: workspace.unwrap_or_default().trim().to_string(),
        application_access: application_access.unwrap_or_default().trim().to_string(),
    };

    if !tokens.workspace.is_empty() && !tokens.application_access.is_empty() {
        return Err(RegistrationGateError::ConflictingInviteKinds);
    }

    Ok(tokens)
}

fn register_href(tokens: &InviteTokens) -> String {
    match tokens.kind() {
        InviteKind::None => "/register".to_string(),
        InviteKind::Workspace => format!("/register?workspace_invite={}", tokens.workspace),
        InviteKind::ApplicationAccess => {
            format!("/register?app_access_invite={}", tokens.application_access)
        }
    }
}

fn login_href(tokens: &InviteTokens) -> String {
    match tokens.kind() {
        InviteKind::None => "/login".to_string(),
        InviteKind::Workspace => format!("/login?workspace_invite={}", tokens.workspace),
        InviteKind::ApplicationAccess => {
            format!("/login?app_access_invite={}", tokens.application_access)
        }
    }
}

fn auth_page_copy(kind: InviteKind, action: &str) -> String {
    match (kind, action) {
        (InviteKind::None, "login") => "Zaloguj się do KSeF Pay".to_string(),
        (InviteKind::None, _) => {
            "Utwórz nowe konto administratora i własny, niezależny workspace".to_string()
        }
        (InviteKind::Workspace, "login") => {
            "Zaloguj się, aby dołączyć do współdzielonego workspace".to_string()
        }
        (InviteKind::Workspace, _) => {
            "Dołącz do istniejącego współdzielonego workspace".to_string()
        }
        (InviteKind::ApplicationAccess, "login") => {
            "Zaloguj się, aby aktywować dostęp do aplikacji".to_string()
        }
        (InviteKind::ApplicationAccess, _) => {
            "Dokończ rejestrację, aby uzyskać dostęp do aplikacji".to_string()
        }
    }
}

fn auth_extra_info(kind: InviteKind, action: &str) -> Option<String> {
    match (kind, action) {
        (InviteKind::None, "register") => Some(
            "Rejestracja bez zaproszenia jest tylko dla bootstrap administratorów skonfigurowanych przez operatora. Taka rejestracja tworzy osobny workspace z własnymi danymi.".to_string(),
        ),
        (InviteKind::Workspace, _) => Some(
            "To zaproszenie dołącza do konkretnego workspace i udostępnia jego dane zgodnie z rolą. Nie jest to tylko dostęp do samej aplikacji.".to_string(),
        ),
        (InviteKind::ApplicationAccess, _) => Some(
            "To zaproszenie daje dostęp do aplikacji, ale nie dołącza do cudzego workspace. Po akceptacji użytkownik pracuje we własnym, niezależnym workspace.".to_string(),
        ),
        _ => None,
    }
}

fn render_login_template(
    status: StatusCode,
    error: Option<String>,
    email: String,
    csrf_token: String,
    tokens: InviteTokens,
    invite_message: Option<String>,
) -> Response {
    let kind = tokens.kind();
    let register_href = register_href(&tokens);
    render_with_status(
        status,
        LoginTemplate {
            error,
            email,
            csrf_token,
            workspace_invite_token: tokens.workspace,
            app_access_invite_token: tokens.application_access,
            invite_message,
            page_copy: auth_page_copy(kind, "login"),
            extra_info: auth_extra_info(kind, "login"),
            register_href,
        },
    )
}

fn render_register_template(
    status: StatusCode,
    error: Option<String>,
    email: String,
    csrf_token: String,
    tokens: InviteTokens,
    invite_message: Option<String>,
) -> Response {
    let kind = tokens.kind();
    let login_href = login_href(&tokens);
    render_with_status(
        status,
        RegisterTemplate {
            error,
            email,
            csrf_token,
            workspace_invite_token: tokens.workspace,
            app_access_invite_token: tokens.application_access,
            invite_message,
            page_copy: auth_page_copy(kind, "register"),
            extra_info: auth_extra_info(kind, "register"),
            login_href,
        },
    )
}

async fn workspace_invite_message(state: &AppState, invite: &WorkspaceInvite) -> String {
    match state.workspace_repo.find_by_id(&invite.workspace_id).await {
        Ok(workspace) => format!(
            "To zaproszenie doda konto {} do workspace {} jako {}. Po akceptacji użytkownik zobaczy dane tego workspace, w tym przypisane NIP-y i faktury zgodnie z rolą.",
            invite.email,
            workspace.display_name,
            invite.role.display_name()
        ),
        Err(_) => format!(
            "To zaproszenie doda konto {} do workspace {} jako {}. Po akceptacji użytkownik zobaczy dane tego workspace zgodnie z rolą.",
            invite.email,
            invite.workspace_id,
            invite.role.display_name()
        ),
    }
}

async fn application_access_invite_message(
    state: &AppState,
    invite: &ApplicationAccessInvite,
) -> String {
    let inviter = state
        .user_repo
        .find_by_id(&invite.created_by_user_id)
        .await
        .map(|user| user.email)
        .unwrap_or_else(|_| invite.created_by_user_id.to_string());

    format!(
        "To zaproszenie da kontu {} dostęp do aplikacji. Po akceptacji użytkownik zaloguje się do KSeF Pay i utworzy własny, niezależny workspace. Zaprasza: {}.",
        invite.email, inviter
    )
}

async fn invite_page_context(
    state: &AppState,
    tokens: &InviteTokens,
) -> (Option<String>, Option<String>, Option<String>) {
    match tokens.kind() {
        InviteKind::None => (None, None, None),
        InviteKind::Workspace => match resolve_pending_invite(state, &tokens.workspace).await {
            Ok(invite) => (
                None,
                Some(invite.email.clone()),
                Some(workspace_invite_message(state, &invite).await),
            ),
            Err(err) => (
                Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                None,
                None,
            ),
        },
        InviteKind::ApplicationAccess => {
            match resolve_pending_application_access_invite(state, &tokens.application_access).await
            {
                Ok(invite) => (
                    None,
                    Some(invite.email.clone()),
                    Some(application_access_invite_message(state, &invite).await),
                ),
                Err(err) => (
                    Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                    None,
                    None,
                ),
            }
        }
    }
}

async fn current_session_user_email(state: &AppState, session: &Session) -> Option<String> {
    let user_id_raw = session.get::<String>("user_id").await.ok().flatten()?;
    let user_id: UserId = user_id_raw.parse().ok()?;
    state.user_repo.find_by_id(&user_id).await.ok().map(|user| user.email)
}

async fn resolve_registration_authorization(
    state: &AppState,
    email: &str,
    tokens: &InviteTokens,
) -> Result<RegistrationAuthorization, RegistrationGateError> {
    match tokens.kind() {
        InviteKind::Workspace => {
            let invite = resolve_pending_invite(state, &tokens.workspace).await?;
            require_invite_email(&invite, email)?;
            Ok(RegistrationAuthorization::WorkspaceInvite(invite))
        }
        InviteKind::ApplicationAccess => {
            let invite =
                resolve_pending_application_access_invite(state, &tokens.application_access)
                    .await?;
            require_application_access_invite_email(&invite, email)?;
            Ok(RegistrationAuthorization::ApplicationAccessInvite(invite))
        }
        InviteKind::None => {
            if state.allowed_emails.contains(&email.to_lowercase()) {
                Ok(RegistrationAuthorization::BootstrapAdmin)
            } else {
                Err(RegistrationGateError::Closed)
            }
        }
    }
}

async fn apply_workspace_invite(
    state: &AppState,
    user: &User,
    invite: &WorkspaceInvite,
) -> Result<ksef_core::domain::workspace::WorkspaceId, RepositoryError> {
    state
        .workspace_repo
        .activate_invite_membership(invite, &user.id)
        .await?;
    Ok(invite.workspace_id.clone())
}

async fn prepare_independent_workspace(
    state: &AppState,
    user: &User,
) -> Result<ksef_core::domain::workspace::WorkspaceId, RepositoryError> {
    let workspace = state
        .workspace_repo
        .ensure_default_workspace(&user.id, &user.email)
        .await?;
    Ok(workspace.workspace.id)
}

async fn apply_application_access_invite(
    state: &AppState,
    user: &User,
    invite: &ApplicationAccessInvite,
) -> Result<ksef_core::domain::workspace::WorkspaceId, RepositoryError> {
    let workspace = state
        .application_access_repo
        .activate_application_access(&invite.id, &user.id, &user.email)
        .await?;
    Ok(workspace.workspace.id)
}

async fn persist_authenticated_session(
    session: &Session,
    user_id: &UserId,
    workspace_id: &ksef_core::domain::workspace::WorkspaceId,
) -> Result<(), RepositoryError> {
    session
        .insert(CURRENT_WORKSPACE_SESSION_KEY, workspace_id.to_string())
        .await
        .map_err(|e| RepositoryError::Storage(format!("session write error: {e}")))?;

    if let Err(err) = session.insert("user_id", user_id.to_string()).await {
        let _ = session.remove::<String>(CURRENT_WORKSPACE_SESSION_KEY).await;
        return Err(RepositoryError::Storage(format!("session write error: {err}")));
    }

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
    let tokens = match normalize_invite_tokens(query.workspace_invite, query.app_access_invite) {
        Ok(tokens) => tokens,
        Err(err) => {
            let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
            return render_login_template(
                StatusCode::BAD_REQUEST,
                Some(err.to_string()),
                String::new(),
                csrf_token,
                InviteTokens::default(),
                None,
            );
        }
    };

    if matches!(tokens.kind(), InviteKind::None) {
        if let Ok(Some(_)) = session.get::<String>("user_id").await {
            return Redirect::to("/accounts").into_response();
        }
    } else if let Some(current_email) = current_session_user_email(&state, &session).await {
        let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
        let (_, invited_email, invited_message) = invite_page_context(&state, &tokens).await;
        let invited_email = invited_email.unwrap_or_default();
        return render_login_template(
            StatusCode::CONFLICT,
            Some(format!(
                "Jesteś już zalogowany jako {}. Ten link otwórz po wylogowaniu albo w oknie incognito, aby nie użyć błędnej sesji.",
                current_email
            )),
            invited_email,
            csrf_token,
            tokens,
            invited_message,
        );
    }

    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let (error, invited_email, invited_message) = invite_page_context(&state, &tokens).await;

    render(LoginTemplate {
        error,
        email: invited_email.unwrap_or_default(),
        csrf_token,
        workspace_invite_token: tokens.workspace.clone(),
        app_access_invite_token: tokens.application_access.clone(),
        invite_message: invited_message,
        page_copy: auth_page_copy(tokens.kind(), "login"),
        extra_info: auth_extra_info(tokens.kind(), "login"),
        register_href: register_href(&tokens),
    })
}

pub async fn login(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    CsrfForm(form): CsrfForm<LoginFormData>,
) -> Response {
    let email = normalize_email(&form.email);
    let tokens =
        match normalize_invite_tokens(form.workspace_invite_token, form.app_access_invite_token) {
            Ok(tokens) => tokens,
            Err(err) => {
                let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
                return render_login_template(
                    StatusCode::BAD_REQUEST,
                    Some(err.to_string()),
                    email,
                    csrf_token,
                    InviteTokens::default(),
                    None,
                );
            }
        };
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
                tokens,
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
            tokens,
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
                tokens,
                None,
            );
        }
        Err(e) => {
            return render_login_template(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(format!("Błąd serwera: {e}")),
                email,
                csrf_token,
                tokens,
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
            tokens,
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
            tokens,
            None,
        );
    }

    let workspace_id = match tokens.kind() {
        InviteKind::None => {
            match prepare_independent_workspace(&state, &user).await {
                Ok(workspace_id) => workspace_id,
                Err(e) => {
                    return render_login_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!("Nie udało się przygotować workspace: {e}")),
                        email,
                        csrf_token,
                        tokens,
                        None,
                    );
                }
            }
        }
        InviteKind::Workspace => {
            let invite = match resolve_pending_invite(&state, &tokens.workspace).await {
                Ok(invite) => invite,
                Err(err) => {
                    return render_login_template(
                        StatusCode::FORBIDDEN,
                        Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                        email,
                        csrf_token,
                        tokens,
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
                    tokens.clone(),
                    Some(workspace_invite_message(&state, &invite).await),
                );
            }
            match apply_workspace_invite(&state, &user, &invite).await {
                Ok(workspace_id) => workspace_id,
                Err(err) => {
                    return render_login_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!("Nie udało się aktywować zaproszenia: {err}")),
                        email,
                        csrf_token,
                        tokens.clone(),
                        Some(workspace_invite_message(&state, &invite).await),
                    );
                }
            }
        }
        InviteKind::ApplicationAccess => {
            let invite =
                match resolve_pending_application_access_invite(&state, &tokens.application_access)
                    .await
                {
                    Ok(invite) => invite,
                    Err(err) => {
                        return render_login_template(
                            StatusCode::FORBIDDEN,
                            Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                            email,
                            csrf_token,
                            tokens,
                            None,
                        );
                    }
                };
            if let Err(err) = require_application_access_invite_email(&invite, &user.email) {
                return render_login_template(
                    StatusCode::FORBIDDEN,
                    Some(format!("Zaproszenie nie pasuje do tego konta: {err}")),
                    email,
                    csrf_token,
                    tokens.clone(),
                    Some(application_access_invite_message(&state, &invite).await),
                );
            }
            match apply_application_access_invite(&state, &user, &invite).await {
                Ok(workspace_id) => workspace_id,
                Err(err) => {
                    return render_login_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!(
                            "Nie udało się aktywować dostępu do aplikacji: {err}"
                        )),
                        email,
                        csrf_token,
                        tokens.clone(),
                        Some(application_access_invite_message(&state, &invite).await),
                    );
                }
            }
        }
    };

    if let Err(e) = persist_authenticated_session(&session, &user.id, &workspace_id).await {
        return render_login_template(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(format!("Błąd sesji: {e}")),
            email,
            csrf_token,
            tokens,
            None,
        );
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
    let tokens = match normalize_invite_tokens(query.workspace_invite, query.app_access_invite) {
        Ok(tokens) => tokens,
        Err(err) => {
            let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
            return render_register_template(
                StatusCode::BAD_REQUEST,
                Some(err.to_string()),
                String::new(),
                csrf_token,
                InviteTokens::default(),
                None,
            );
        }
    };

    if matches!(tokens.kind(), InviteKind::None) {
        if let Ok(Some(_)) = session.get::<String>("user_id").await {
            return Redirect::to("/accounts").into_response();
        }
    } else if let Some(current_email) = current_session_user_email(&state, &session).await {
        let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
        let (_, invited_email, invited_message) = invite_page_context(&state, &tokens).await;
        let invited_email = invited_email.unwrap_or_default();
        return render_register_template(
            StatusCode::CONFLICT,
            Some(format!(
                "Jesteś już zalogowany jako {}. Ten link otwórz po wylogowaniu albo w oknie incognito, aby nie użyć błędnej sesji.",
                current_email
            )),
            invited_email,
            csrf_token,
            tokens,
            invited_message,
        );
    }

    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let (error, invited_email, invited_message) = invite_page_context(&state, &tokens).await;

    render(RegisterTemplate {
        error,
        email: invited_email.unwrap_or_default(),
        csrf_token,
        workspace_invite_token: tokens.workspace.clone(),
        app_access_invite_token: tokens.application_access.clone(),
        invite_message: invited_message,
        page_copy: auth_page_copy(tokens.kind(), "register"),
        extra_info: auth_extra_info(tokens.kind(), "register"),
        login_href: login_href(&tokens),
    })
}

pub async fn register(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    CsrfForm(form): CsrfForm<RegisterFormData>,
) -> Response {
    let email = normalize_email(&form.email);
    let tokens =
        match normalize_invite_tokens(form.workspace_invite_token, form.app_access_invite_token) {
            Ok(tokens) => tokens,
            Err(err) => {
                let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
                return render_register_template(
                    StatusCode::BAD_REQUEST,
                    Some(err.to_string()),
                    email,
                    csrf_token,
                    InviteTokens::default(),
                    None,
                );
            }
        };
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
                tokens,
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
            tokens,
            None,
        );
    }

    if !is_valid_email(&email) {
        return render_register_template(
            StatusCode::BAD_REQUEST,
            Some("Nieprawidłowy adres email".to_string()),
            email,
            csrf_token,
            tokens,
            None,
        );
    }

    let authorization = match resolve_registration_authorization(&state, &email, &tokens).await {
        Ok(authorization) => authorization,
        Err(RegistrationGateError::Closed) => {
            return render_register_template(
                StatusCode::FORBIDDEN,
                Some(
                    "Rejestracja jest zamknięta. Poproś administratora o zaproszenie do aplikacji albo administratora workspace o zaproszenie do współdzielonego workspace."
                        .to_string(),
                ),
                email,
                csrf_token,
                tokens,
                None,
            );
        }
        Err(RegistrationGateError::WorkspaceInvite(err)) => {
            return render_register_template(
                StatusCode::FORBIDDEN,
                Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                email,
                csrf_token,
                tokens,
                None,
            );
        }
        Err(RegistrationGateError::ApplicationAccessInvite(err)) => {
            return render_register_template(
                StatusCode::FORBIDDEN,
                Some(format!("Zaproszenie jest nieprawidłowe: {err}")),
                email,
                csrf_token,
                tokens,
                None,
            );
        }
        Err(RegistrationGateError::ConflictingInviteKinds) => {
            return render_register_template(
                StatusCode::BAD_REQUEST,
                Some("Nie można użyć dwóch typów zaproszenia jednocześnie.".to_string()),
                email,
                csrf_token,
                tokens,
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
            tokens,
            None,
        );
    }

    if form.password != form.password_confirm {
        return render_register_template(
            StatusCode::BAD_REQUEST,
            Some("Hasła nie są zgodne".to_string()),
            email,
            csrf_token,
            tokens,
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
                tokens,
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
                tokens,
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
                tokens,
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
            tokens,
            None,
        );
    }

    let workspace_id = match authorization {
        RegistrationAuthorization::BootstrapAdmin => {
            match prepare_independent_workspace(&state, &user).await {
                Ok(workspace_id) => workspace_id,
                Err(e) => {
                    return render_register_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!("Nie udało się przygotować workspace: {e}")),
                        email,
                        csrf_token,
                        tokens,
                        None,
                    );
                }
            }
        }
        RegistrationAuthorization::WorkspaceInvite(invite) => {
            let invite_info = workspace_invite_message(&state, &invite).await;
            match apply_workspace_invite(&state, &user, &invite).await {
                Ok(workspace_id) => workspace_id,
                Err(e) => {
                    return render_register_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!("Nie udało się aktywować zaproszenia: {e}")),
                        email,
                        csrf_token,
                        tokens,
                        Some(invite_info),
                    );
                }
            }
        }
        RegistrationAuthorization::ApplicationAccessInvite(invite) => {
            let invite_info = application_access_invite_message(&state, &invite).await;
            match apply_application_access_invite(&state, &user, &invite).await {
                Ok(workspace_id) => workspace_id,
                Err(e) => {
                    return render_register_template(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(format!("Nie udało się aktywować dostępu do aplikacji: {e}")),
                        email,
                        csrf_token,
                        tokens,
                        Some(invite_info),
                    );
                }
            }
        }
    };

    if let Err(e) = persist_authenticated_session(&session, &user.id, &workspace_id).await {
        return render_register_template(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(format!("Błąd sesji: {e}")),
            email,
            csrf_token,
            tokens,
            None,
        );
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
    use tower_sessions::{MemoryStore, Session};

    use ksef_core::domain::application_access::{
        ApplicationAccessInvite, ApplicationAccessInviteId,
    };
    use ksef_core::domain::environment::KSeFEnvironment;
    use ksef_core::domain::workspace::{
        WorkspaceInviteId, WorkspaceMembershipStatus, WorkspaceRole,
    };
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

    use crate::auth_rate_limit::AuthRateLimiter;
    use crate::email::NoopEmailSender;
    use crate::invite_tokens::hash_invite_token;
    use chrono::Duration;

    async fn test_state(allowed_emails: Vec<String>) -> (AppState, std::path::PathBuf) {
        let db_path =
            std::env::temp_dir().join(format!("ksef-server-auth-test-{}.db", uuid::Uuid::new_v4()));
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
        let qr_service = Arc::new(QRService::new(
            KSeFEnvironment::Production,
            qr_renderer.clone(),
        ));
        let offline_service = Arc::new(OfflineService::new(
            QRService::new(KSeFEnvironment::Production, qr_renderer),
            OfflineConfig::default(),
        ));

        let state = AppState {
            ksef_environment: KSeFEnvironment::Production,
            user_repo: db.user_repo.clone(),
            nip_account_repo: db.nip_account_repo.clone(),
            workspace_repo: db.workspace_repo.clone(),
            application_access_repo: db.application_access_repo.clone(),
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
                token_hash: hash_invite_token(raw_token),
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

    async fn create_application_access_invite(
        state: &AppState,
        inviter: &User,
        invited_email: &str,
        raw_token: &str,
    ) {
        state
            .application_access_repo
            .create_invite(&ApplicationAccessInvite {
                id: ApplicationAccessInviteId::new(),
                email: invited_email.to_string(),
                token_hash: hash_invite_token(raw_token),
                expires_at: Utc::now() + Duration::days(7),
                accepted_at: None,
                revoked_at: None,
                created_by_user_id: inviter.id.clone(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
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
                workspace_invite_token: None,
                app_access_invite_token: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_text(response).await;
        assert!(body.contains("administratora o zaproszenie do aplikacji"));

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
                workspace_invite_token: None,
                app_access_invite_token: None,
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
    async fn register_with_valid_workspace_invite_creates_membership() {
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
                workspace_invite_token: Some(raw_token.to_string()),
                app_access_invite_token: None,
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
        assert_eq!(membership.status, WorkspaceMembershipStatus::Active);

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn register_with_application_access_invite_creates_independent_workspace() {
        let (state, db_path) = test_state(Vec::new()).await;
        let inviter = make_user("admin@example.com", "AdminPass1!");
        state.user_repo.create(&inviter).await.unwrap();
        let inviter_workspace = state
            .workspace_repo
            .ensure_default_workspace(&inviter.id, &inviter.email)
            .await
            .unwrap()
            .workspace;
        let raw_token = "app.access.register.token";
        create_application_access_invite(&state, &inviter, "new.app.user@example.com", raw_token)
            .await;

        let response = register(
            State(state.clone()),
            session_with_csrf("csrf").await,
            HeaderMap::new(),
            CsrfForm(RegisterFormData {
                email: "new.app.user@example.com".to_string(),
                password: "Passw0rd!".to_string(),
                password_confirm: "Passw0rd!".to_string(),
                workspace_invite_token: None,
                app_access_invite_token: Some(raw_token.to_string()),
            }),
        )
        .await;

        assert!(response.status().is_redirection());
        let user = state
            .user_repo
            .find_by_email("new.app.user@example.com")
            .await
            .unwrap()
            .unwrap();
        let workspaces = state.workspace_repo.list_for_user(&user.id).await.unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].membership.role, WorkspaceRole::Owner);
        assert_eq!(workspaces[0].workspace.created_by_user_id, user.id);
        let membership = state
            .workspace_repo
            .find_membership(&inviter_workspace.id, &user.id)
            .await
            .unwrap();
        assert!(membership.is_none());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn login_with_valid_workspace_invite_grants_existing_user_membership() {
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
                workspace_invite_token: Some(raw_token.to_string()),
                app_access_invite_token: None,
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

    #[tokio::test]
    async fn login_with_invalid_invite_does_not_create_session() {
        let (state, db_path) = test_state(Vec::new()).await;
        let user = make_user("existing.user@example.com", "Passw0rd!");
        state.user_repo.create(&user).await.unwrap();
        let session = session_with_csrf("csrf").await;

        let response = login(
            State(state),
            session.clone(),
            HeaderMap::new(),
            CsrfForm(LoginFormData {
                email: "existing.user@example.com".to_string(),
                password: "Passw0rd!".to_string(),
                workspace_invite_token: Some("missing-invite".to_string()),
                app_access_invite_token: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(session.get::<String>("user_id").await.unwrap().is_none());
        assert!(
            session
                .get::<String>(CURRENT_WORKSPACE_SESSION_KEY)
                .await
                .unwrap()
                .is_none()
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn register_page_with_invite_and_active_other_session_shows_conflict() {
        let (state, db_path) = test_state(Vec::new()).await;
        let active_user = make_user("active@example.com", "Passw0rd!");
        state.user_repo.create(&active_user).await.unwrap();
        let inviter = make_user("owner@example.com", "OwnerPass1!");
        state.user_repo.create(&inviter).await.unwrap();
        let raw_token = "app.access.page.token";
        create_application_access_invite(&state, &inviter, "invited@example.com", raw_token).await;

        let session = session_with_csrf("csrf").await;
        session
            .insert("user_id", active_user.id.to_string())
            .await
            .unwrap();

        let response = register_page(
            State(state.clone()),
            session,
            Query(InviteQuery {
                workspace_invite: None,
                app_access_invite: Some(raw_token.to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response_text(response).await;
        assert!(body.contains("Jesteś już zalogowany jako active@example.com"));
        assert!(body.contains("oknie incognito"));

        let _ = std::fs::remove_file(db_path);
    }
}
