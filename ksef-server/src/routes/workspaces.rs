use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;
use tower_sessions::Session;
use uuid::Uuid;

use ksef_core::domain::workspace::{
    Workspace, WorkspaceId, WorkspaceInvite, WorkspaceInviteId, WorkspaceMembershipStatus,
    WorkspaceRole,
};
use ksef_core::error::RepositoryError;

use crate::csrf::ensure_csrf_token;
use crate::email::{EmailSendError, WorkspaceInviteEmail, dispatch_workspace_invite};
use crate::extractors::{AuthUser, CURRENT_WORKSPACE_SESSION_KEY, CsrfForm, WorkspaceContext};
use crate::invite_tokens::{generate_invite_token, hash_invite_token, invite_expiration};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SelectWorkspaceFormData {
    pub workspace_id: String,
    pub return_to: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateInviteFormData {
    pub email: String,
    pub role: String,
}

#[derive(Deserialize)]
pub struct RevokeInviteFormData {}

#[derive(Deserialize)]
pub struct CreateWorkspaceFormData {
    pub display_name: String,
}

#[derive(Debug, Clone)]
struct PendingInviteTemplate {
    id: String,
    email: String,
    role_label: &'static str,
    expires_at: String,
}

#[derive(Template)]
#[template(path = "pages/workspace_access.html")]
struct WorkspaceAccessTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    current_workspace_name: String,
    csrf_token: String,
    invite_email: String,
    selected_role: String,
    pending_invites: Vec<PendingInviteTemplate>,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/workspace_new.html")]
struct WorkspaceNewTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    current_workspace_name: String,
    csrf_token: String,
    display_name: String,
    error: Option<String>,
}

#[derive(Debug, Error)]
enum SelectWorkspaceError {
    #[error("invalid workspace id")]
    InvalidWorkspaceId,
    #[error("forbidden workspace access")]
    Forbidden,
    #[error("workspace repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("session write error: {0}")]
    Session(String),
}

#[derive(Debug, Error)]
enum WorkspaceAccessError {
    #[error("forbidden workspace management")]
    Forbidden,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("invalid workspace role")]
    InvalidRole,
    #[error("invite already pending for this email")]
    DuplicateInvite,
    #[error("user already has active workspace access")]
    AlreadyMember,
    #[error("invite not found in current workspace")]
    InviteNotFound,
    #[error("invite email delivery failed: {0}")]
    InviteDelivery(#[from] EmailSendError),
    #[error("workspace repository error: {0}")]
    Repository(#[from] RepositoryError),
}

#[derive(Debug, Error)]
enum WorkspaceCreateError {
    #[error("workspace name is required")]
    MissingName,
    #[error("workspace name is too short")]
    NameTooShort,
    #[error("workspace repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("session write error: {0}")]
    Session(String),
}

impl IntoResponse for SelectWorkspaceError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidWorkspaceId => (
                axum::http::StatusCode::BAD_REQUEST,
                "Nieprawidłowy identyfikator workspace",
            )
                .into_response(),
            Self::Forbidden => (
                axum::http::StatusCode::FORBIDDEN,
                "Brak dostępu do wybranego workspace",
            )
                .into_response(),
            Self::Repository(err) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udało się przełączyć workspace: {err}"),
            )
                .into_response(),
            Self::Session(msg) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

fn render<T: Template>(tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {e}"),
        )
            .into_response(),
    }
}

fn render_workspace_access(
    status: axum::http::StatusCode,
    workspace_ctx: &WorkspaceContext,
    csrf_token: String,
    pending_invites: Vec<PendingInviteTemplate>,
    invite_email: String,
    selected_role: String,
    error: Option<String>,
    success: Option<String>,
) -> Response {
    match (WorkspaceAccessTemplate {
        active: "/workspace-access",
        nip_prefix: None,
        user_email: workspace_ctx.user.email.clone(),
        current_workspace_name: workspace_ctx.workspace.display_name.clone(),
        csrf_token,
        invite_email,
        selected_role,
        pending_invites,
        error,
        success,
    })
    .render()
    {
        Ok(html) => (status, Html(html)).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {e}"),
        )
            .into_response(),
    }
}

fn render_workspace_new(
    status: axum::http::StatusCode,
    workspace_ctx: &WorkspaceContext,
    csrf_token: String,
    display_name: String,
    error: Option<String>,
) -> Response {
    match (WorkspaceNewTemplate {
        active: "/workspace-new",
        nip_prefix: None,
        user_email: workspace_ctx.user.email.clone(),
        current_workspace_name: workspace_ctx.workspace.display_name.clone(),
        csrf_token,
        display_name,
        error,
    })
    .render()
    {
        Ok(html) => (status, Html(html)).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {e}"),
        )
            .into_response(),
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn normalize_workspace_name(display_name: &str) -> String {
    display_name.trim().to_string()
}

fn workspace_slug(display_name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;

    for ch in display_name.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    let base = if slug.is_empty() {
        "workspace".to_string()
    } else {
        slug
    };
    let suffix: String = Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();
    format!("{base}-{suffix}")
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

fn pending_invite_template(invite: WorkspaceInvite) -> PendingInviteTemplate {
    PendingInviteTemplate {
        id: invite.id.to_string(),
        email: invite.email,
        role_label: invite.role.display_name(),
        expires_at: invite.expires_at.format("%Y-%m-%d %H:%M UTC").to_string(),
    }
}

async fn load_pending_invites(
    state: &AppState,
    workspace_id: &WorkspaceId,
) -> Result<Vec<PendingInviteTemplate>, RepositoryError> {
    state
        .workspace_repo
        .list_pending_invites(workspace_id)
        .await
        .map(|invites| invites.into_iter().map(pending_invite_template).collect())
}

fn parse_invite_role(raw_role: &str) -> Result<WorkspaceRole, WorkspaceAccessError> {
    match raw_role.trim().to_ascii_lowercase().as_str() {
        "admin" => Ok(WorkspaceRole::Admin),
        "operator" => Ok(WorkspaceRole::Operator),
        "read_only" => Ok(WorkspaceRole::ReadOnly),
        _ => Err(WorkspaceAccessError::InvalidRole),
    }
}

fn invite_url(base_url: &str, raw_token: &str) -> String {
    format!(
        "{}/register?workspace_invite={raw_token}",
        base_url.trim_end_matches('/')
    )
}

async fn ensure_manage_members(
    workspace_ctx: &WorkspaceContext,
) -> Result<(), WorkspaceAccessError> {
    if workspace_ctx.membership.can_manage_members {
        Ok(())
    } else {
        Err(WorkspaceAccessError::Forbidden)
    }
}

pub async fn select(
    State(state): State<AppState>,
    auth: AuthUser,
    session: Session,
    CsrfForm(form): CsrfForm<SelectWorkspaceFormData>,
) -> Response {
    match try_select_workspace(state, auth, session, form).await {
        Ok(target) => Redirect::to(&target).into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn access_page(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
) -> Response {
    if let Err(err) = ensure_manage_members(&workspace_ctx).await {
        return (axum::http::StatusCode::FORBIDDEN, err.to_string()).into_response();
    }

    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let pending_invites = match load_pending_invites(&state, &workspace_ctx.workspace.id).await {
        Ok(invites) => invites,
        Err(err) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udało się pobrać zaproszeń: {err}"),
            )
                .into_response();
        }
    };

    render(WorkspaceAccessTemplate {
        active: "/workspace-access",
        nip_prefix: None,
        user_email: workspace_ctx.user.email,
        current_workspace_name: workspace_ctx.workspace.display_name,
        csrf_token,
        invite_email: String::new(),
        selected_role: "operator".to_string(),
        pending_invites,
        error: None,
        success: None,
    })
}

pub async fn new_page(workspace_ctx: WorkspaceContext, session: Session) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    render_workspace_new(
        axum::http::StatusCode::OK,
        &workspace_ctx,
        csrf_token,
        String::new(),
        None,
    )
}

pub async fn create_workspace(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
    CsrfForm(form): CsrfForm<CreateWorkspaceFormData>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let display_name = normalize_workspace_name(&form.display_name);

    let result: Result<WorkspaceId, WorkspaceCreateError> = async {
        if display_name.is_empty() {
            return Err(WorkspaceCreateError::MissingName);
        }
        if display_name.chars().count() < 3 {
            return Err(WorkspaceCreateError::NameTooShort);
        }

        let now = Utc::now();
        let workspace = Workspace {
            id: WorkspaceId::new(),
            slug: workspace_slug(&display_name),
            display_name: display_name.clone(),
            created_by_user_id: workspace_ctx.user.id.clone(),
            created_at: now,
            updated_at: now,
        };
        let workspace_id = state
            .workspace_repo
            .create_workspace(&workspace, &workspace_ctx.user.id)
            .await?;
        session
            .insert(CURRENT_WORKSPACE_SESSION_KEY, workspace_id.to_string())
            .await
            .map_err(|e| WorkspaceCreateError::Session(format!("session write error: {e}")))?;
        Ok(workspace_id)
    }
    .await;

    match result {
        Ok(_) => Redirect::to("/accounts").into_response(),
        Err(err @ WorkspaceCreateError::MissingName)
        | Err(err @ WorkspaceCreateError::NameTooShort) => render_workspace_new(
            axum::http::StatusCode::BAD_REQUEST,
            &workspace_ctx,
            csrf_token,
            display_name,
            Some(err.to_string()),
        ),
        Err(err) => render_workspace_new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            csrf_token,
            display_name,
            Some(err.to_string()),
        ),
    }
}

pub async fn create_invite(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
    CsrfForm(form): CsrfForm<CreateInviteFormData>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let pending_invites = load_pending_invites(&state, &workspace_ctx.workspace.id)
        .await
        .unwrap_or_default();
    let email = normalize_email(&form.email);
    let selected_role = form.role.clone();

    if let Err(err) = ensure_manage_members(&workspace_ctx).await {
        return render_workspace_access(
            axum::http::StatusCode::FORBIDDEN,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            email,
            selected_role,
            Some(err.to_string()),
            None,
        );
    }

    let role = match parse_invite_role(&form.role) {
        Ok(role) => role,
        Err(err) => {
            return render_workspace_access(
                axum::http::StatusCode::BAD_REQUEST,
                &workspace_ctx,
                csrf_token,
                pending_invites,
                email,
                selected_role,
                Some(err.to_string()),
                None,
            );
        }
    };

    if !is_valid_email(&email) {
        return render_workspace_access(
            axum::http::StatusCode::BAD_REQUEST,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            email,
            selected_role,
            Some(WorkspaceAccessError::InvalidEmail.to_string()),
            None,
        );
    }

    if pending_invites.iter().any(|invite| invite.email == email) {
        return render_workspace_access(
            axum::http::StatusCode::CONFLICT,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            email,
            selected_role,
            Some(WorkspaceAccessError::DuplicateInvite.to_string()),
            None,
        );
    }

    match state.user_repo.find_by_email(&email).await {
        Ok(Some(user)) => match state
            .workspace_repo
            .find_membership(&workspace_ctx.workspace.id, &user.id)
            .await
        {
            Ok(Some(membership)) if membership.status == WorkspaceMembershipStatus::Active => {
                return render_workspace_access(
                    axum::http::StatusCode::CONFLICT,
                    &workspace_ctx,
                    csrf_token,
                    pending_invites,
                    email,
                    selected_role,
                    Some(WorkspaceAccessError::AlreadyMember.to_string()),
                    None,
                );
            }
            Ok(_) => {}
            Err(err) => {
                return render_workspace_access(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &workspace_ctx,
                    csrf_token,
                    pending_invites,
                    email,
                    selected_role,
                    Some(format!("Nie udało się sprawdzić membership: {err}")),
                    None,
                );
            }
        },
        Ok(None) => {}
        Err(err) => {
            return render_workspace_access(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &workspace_ctx,
                csrf_token,
                pending_invites,
                email,
                selected_role,
                Some(format!("Nie udało się sprawdzić użytkownika: {err}")),
                None,
            );
        }
    }

    let raw_token = generate_invite_token();
    let invite = WorkspaceInvite {
        id: WorkspaceInviteId::new(),
        workspace_id: workspace_ctx.workspace.id.clone(),
        email: email.clone(),
        role,
        token_hash: hash_invite_token(&raw_token),
        expires_at: invite_expiration(),
        accepted_at: None,
        revoked_at: None,
        created_by_user_id: workspace_ctx.user.id.clone(),
        created_at: Utc::now(),
    };

    if let Err(err) = state.workspace_repo.create_invite(&invite).await {
        return render_workspace_access(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            email,
            selected_role,
            Some(format!("Nie udało się utworzyć zaproszenia: {err}")),
            None,
        );
    }

    let invite_email = WorkspaceInviteEmail {
        recipient_email: invite.email.clone(),
        workspace_name: workspace_ctx.workspace.display_name.clone(),
        inviter_email: workspace_ctx.user.email.clone(),
        role_label: invite.role.display_name().to_string(),
        invite_url: invite_url(&state.public_base_url, &raw_token),
    };
    if let Err(err) = dispatch_workspace_invite(state.email_sender.clone(), invite_email).await {
        let _ = state.workspace_repo.revoke_invite(&invite.id).await;
        let pending_invites = load_pending_invites(&state, &workspace_ctx.workspace.id)
            .await
            .unwrap_or_default();
        return render_workspace_access(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            email,
            selected_role,
            Some(format!("Nie udało się wysłać zaproszenia email: {err}")),
            None,
        );
    }

    let pending_invites = load_pending_invites(&state, &workspace_ctx.workspace.id)
        .await
        .unwrap_or_default();

    render_workspace_access(
        axum::http::StatusCode::OK,
        &workspace_ctx,
        csrf_token,
        pending_invites,
        String::new(),
        "operator".to_string(),
        None,
        Some(format!("Wysłano email z zaproszeniem do {}.", invite.email)),
    )
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
    Path(invite_id_raw): Path<String>,
    CsrfForm(_form): CsrfForm<RevokeInviteFormData>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let pending_invites = load_pending_invites(&state, &workspace_ctx.workspace.id)
        .await
        .unwrap_or_default();

    if let Err(err) = ensure_manage_members(&workspace_ctx).await {
        return render_workspace_access(
            axum::http::StatusCode::FORBIDDEN,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            String::new(),
            "operator".to_string(),
            Some(err.to_string()),
            None,
        );
    }

    let invite_id: WorkspaceInviteId = match invite_id_raw.parse() {
        Ok(invite_id) => invite_id,
        Err(_) => {
            return render_workspace_access(
                axum::http::StatusCode::BAD_REQUEST,
                &workspace_ctx,
                csrf_token,
                pending_invites,
                String::new(),
                "operator".to_string(),
                Some("Nieprawidłowy identyfikator zaproszenia.".to_string()),
                None,
            );
        }
    };

    if !state
        .workspace_repo
        .list_pending_invites(&workspace_ctx.workspace.id)
        .await
        .unwrap_or_default()
        .iter()
        .any(|invite| invite.id == invite_id)
    {
        return render_workspace_access(
            axum::http::StatusCode::NOT_FOUND,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            String::new(),
            "operator".to_string(),
            Some(WorkspaceAccessError::InviteNotFound.to_string()),
            None,
        );
    }

    if let Err(err) = state.workspace_repo.revoke_invite(&invite_id).await {
        return render_workspace_access(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            csrf_token,
            pending_invites,
            String::new(),
            "operator".to_string(),
            Some(format!("Nie udało się odwołać zaproszenia: {err}")),
            None,
        );
    }

    let pending_invites = load_pending_invites(&state, &workspace_ctx.workspace.id)
        .await
        .unwrap_or_default();
    render_workspace_access(
        axum::http::StatusCode::OK,
        &workspace_ctx,
        csrf_token,
        pending_invites,
        String::new(),
        "operator".to_string(),
        None,
        Some("Zaproszenie zostało odwołane.".to_string()),
    )
}

async fn try_select_workspace(
    state: AppState,
    auth: AuthUser,
    session: Session,
    form: SelectWorkspaceFormData,
) -> Result<String, SelectWorkspaceError> {
    let workspace_id: WorkspaceId = form
        .workspace_id
        .parse()
        .map_err(|_| SelectWorkspaceError::InvalidWorkspaceId)?;

    let membership = state
        .workspace_repo
        .find_membership(&workspace_id, &auth.id)
        .await?;
    match membership {
        Some(membership) if membership.status == WorkspaceMembershipStatus::Active => {}
        _ => return Err(SelectWorkspaceError::Forbidden),
    }

    session
        .insert(CURRENT_WORKSPACE_SESSION_KEY, workspace_id.to_string())
        .await
        .map_err(|e| SelectWorkspaceError::Session(format!("Błąd sesji: {e}")))?;

    Ok(form.return_to.unwrap_or_else(|| "/accounts".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use tower_sessions::{MemoryStore, Session};

    use ksef_core::domain::environment::KSeFEnvironment;
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

    use crate::auth_rate_limit::AuthRateLimiter;
    use crate::email::{
        ApplicationAccessInviteEmail, EmailSendError, EmailSender, WorkspaceInviteEmail,
    };

    #[derive(Default)]
    struct RecordingEmailSender {
        sent: Mutex<Vec<WorkspaceInviteEmail>>,
        fail: bool,
    }

    impl RecordingEmailSender {
        fn sent(&self) -> Vec<WorkspaceInviteEmail> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl EmailSender for RecordingEmailSender {
        fn send_workspace_invite(
            &self,
            invite: WorkspaceInviteEmail,
        ) -> Result<(), EmailSendError> {
            if self.fail {
                return Err(EmailSendError::Transport("smtp failed".to_string()));
            }
            self.sent.lock().unwrap().push(invite);
            Ok(())
        }

        fn send_application_access_invite(
            &self,
            _invite: ApplicationAccessInviteEmail,
        ) -> Result<(), EmailSendError> {
            Ok(())
        }
    }

    async fn test_state(email_sender: Arc<dyn EmailSender>) -> (AppState, std::path::PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "ksef-server-workspaces-test-{}.db",
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
            email_sender,
            export_keys: Arc::new(Mutex::new(HashMap::new())),
            fetch_jobs: Arc::new(Mutex::new(HashMap::new())),
            auth_rate_limiter: AuthRateLimiter::default(),
            public_base_url: "https://app.example.test".to_string(),
            allowed_emails: vec!["admin@example.com".to_string()],
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

    fn make_user(email: &str) -> User {
        let now = Utc::now();
        User {
            id: UserId::new(),
            email: email.to_string(),
            password_hash: "hash".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    async fn workspace_ctx(state: &AppState, user: &User, role: WorkspaceRole) -> WorkspaceContext {
        let summary = state
            .workspace_repo
            .ensure_default_workspace(&user.id, &user.email)
            .await
            .unwrap();
        if role != WorkspaceRole::Owner {
            state
                .workspace_repo
                .add_member(&summary.workspace.id, &user.id, role)
                .await
                .unwrap();
        }
        let membership = state
            .workspace_repo
            .find_membership(&summary.workspace.id, &user.id)
            .await
            .unwrap()
            .unwrap();
        WorkspaceContext {
            user: AuthUser {
                id: user.id.clone(),
                email: user.email.clone(),
            },
            workspace: summary.workspace,
            membership,
        }
    }

    #[tokio::test]
    async fn owner_can_create_workspace_invite() {
        let sender = Arc::new(RecordingEmailSender::default());
        let (state, db_path) = test_state(sender.clone()).await;
        let owner = make_user("owner@example.com");
        state.user_repo.create(&owner).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &owner, WorkspaceRole::Owner).await;

        let response = create_invite(
            State(state.clone()),
            workspace_ctx.clone(),
            session_with_csrf("csrf").await,
            CsrfForm(CreateInviteFormData {
                email: "new.user@example.com".to_string(),
                role: "operator".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("Wysłano email z zaproszeniem"));
        let invites = state
            .workspace_repo
            .list_pending_invites(&workspace_ctx.workspace.id)
            .await
            .unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].email, "new.user@example.com");
        let sent = sender.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].recipient_email, "new.user@example.com");
        assert!(
            sent[0]
                .invite_url
                .starts_with("https://app.example.test/register?workspace_invite=")
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn operator_cannot_create_workspace_invite() {
        let (state, db_path) = test_state(Arc::new(RecordingEmailSender::default())).await;
        let owner = make_user("owner@example.com");
        state.user_repo.create(&owner).await.unwrap();
        let workspace = state
            .workspace_repo
            .ensure_default_workspace(&owner.id, &owner.email)
            .await
            .unwrap()
            .workspace;

        let operator = make_user("operator@example.com");
        state.user_repo.create(&operator).await.unwrap();
        state
            .workspace_repo
            .add_member(&workspace.id, &operator.id, WorkspaceRole::Operator)
            .await
            .unwrap();
        let workspace_ctx = WorkspaceContext {
            user: AuthUser {
                id: operator.id.clone(),
                email: operator.email.clone(),
            },
            workspace: workspace.clone(),
            membership: state
                .workspace_repo
                .find_membership(&workspace.id, &operator.id)
                .await
                .unwrap()
                .unwrap(),
        };

        let response = create_invite(
            State(state),
            workspace_ctx,
            session_with_csrf("csrf").await,
            CsrfForm(CreateInviteFormData {
                email: "new.user@example.com".to_string(),
                role: "operator".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_text(response).await;
        assert!(body.contains("forbidden workspace management"));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn failed_email_delivery_revokes_invite() {
        let sender = Arc::new(RecordingEmailSender {
            sent: Mutex::new(Vec::new()),
            fail: true,
        });
        let (state, db_path) = test_state(sender).await;
        let owner = make_user("owner@example.com");
        state.user_repo.create(&owner).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &owner, WorkspaceRole::Owner).await;

        let response = create_invite(
            State(state.clone()),
            workspace_ctx.clone(),
            session_with_csrf("csrf").await,
            CsrfForm(CreateInviteFormData {
                email: "new.user@example.com".to_string(),
                role: "operator".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let invites = state
            .workspace_repo
            .list_pending_invites(&workspace_ctx.workspace.id)
            .await
            .unwrap();
        assert!(invites.is_empty());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn create_workspace_creates_independent_workspace_and_switches_session() {
        let (state, db_path) = test_state(Arc::new(RecordingEmailSender::default())).await;
        let owner = make_user("owner@example.com");
        state.user_repo.create(&owner).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &owner, WorkspaceRole::Owner).await;
        let original_workspace_id = workspace_ctx.workspace.id.clone();
        let session = session_with_csrf("csrf").await;

        let response = create_workspace(
            State(state.clone()),
            workspace_ctx,
            session.clone(),
            CsrfForm(CreateWorkspaceFormData {
                display_name: "Biuro Alfa".to_string(),
            }),
        )
        .await;

        assert!(response.status().is_redirection());
        let workspaces = state.workspace_repo.list_for_user(&owner.id).await.unwrap();
        assert_eq!(workspaces.len(), 2);
        assert!(
            workspaces
                .iter()
                .any(|summary| summary.workspace.display_name == "Biuro Alfa")
        );

        let selected_workspace_id: String = session
            .get(CURRENT_WORKSPACE_SESSION_KEY)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(selected_workspace_id, original_workspace_id.to_string());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn create_workspace_rejects_blank_name() {
        let (state, db_path) = test_state(Arc::new(RecordingEmailSender::default())).await;
        let owner = make_user("owner@example.com");
        state.user_repo.create(&owner).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &owner, WorkspaceRole::Owner).await;

        let response = create_workspace(
            State(state),
            workspace_ctx,
            session_with_csrf("csrf").await,
            CsrfForm(CreateWorkspaceFormData {
                display_name: "  ".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_text(response).await;
        assert!(body.contains("workspace name is required"));

        let _ = std::fs::remove_file(db_path);
    }
}
