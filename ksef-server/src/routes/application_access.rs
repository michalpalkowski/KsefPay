use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;
use tower_sessions::Session;

use ksef_core::domain::application_access::{ApplicationAccessInvite, ApplicationAccessInviteId};

use crate::csrf::ensure_csrf_token;
use crate::email::{
    ApplicationAccessInviteEmail, EmailSendError, dispatch_application_access_invite,
};
use crate::extractors::{CsrfForm, WorkspaceContext};
use crate::invite_tokens::{generate_invite_token, hash_invite_token, invite_expiration};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateApplicationAccessInviteFormData {
    pub email: String,
}

#[derive(Deserialize)]
pub struct RevokeApplicationAccessInviteFormData {}

#[derive(Debug, Clone)]
struct PendingApplicationAccessInviteTemplate {
    id: String,
    email: String,
    expires_at: String,
    created_by_email: String,
}

#[derive(Template)]
#[template(path = "pages/application_access.html")]
struct ApplicationAccessTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    current_workspace_name: String,
    can_manage_application_access: bool,
    csrf_token: String,
    invite_email: String,
    pending_invites: Vec<PendingApplicationAccessInviteTemplate>,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Debug, Error)]
enum ApplicationAccessError {
    #[error(
        "Tylko bootstrap administrator z ALLOWED_EMAILS może nadawać dostęp do aplikacji. Jeśli chcesz współdzielić dane, użyj sekcji Workspace."
    )]
    Forbidden,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("application access invite already pending for this email")]
    DuplicateInvite,
    #[error("application access invite not found")]
    InviteNotFound,
    #[error("application access email delivery failed: {0}")]
    InviteDelivery(#[from] EmailSendError),
}

fn render<T: Template>(status: axum::http::StatusCode, tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {err}"),
        )
            .into_response(),
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
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

fn invite_url(base_url: &str, raw_token: &str) -> String {
    format!(
        "{}/register?app_access_invite={raw_token}",
        base_url.trim_end_matches('/')
    )
}

fn is_bootstrap_admin(state: &AppState, user_email: &str) -> bool {
    state
        .allowed_emails
        .iter()
        .any(|allowed| allowed == user_email)
}

async fn ensure_bootstrap_admin(
    state: &AppState,
    workspace_ctx: &WorkspaceContext,
) -> Result<(), ApplicationAccessError> {
    if is_bootstrap_admin(state, &workspace_ctx.user.email) {
        Ok(())
    } else {
        Err(ApplicationAccessError::Forbidden)
    }
}

async fn load_pending_invites(
    state: &AppState,
) -> Result<Vec<PendingApplicationAccessInviteTemplate>, String> {
    let invites = state
        .application_access_repo
        .list_pending_invites()
        .await
        .map_err(|err| format!("Nie udało się pobrać zaproszeń: {err}"))?;

    let mut templates = Vec::with_capacity(invites.len());
    for invite in invites {
        let created_by_email = state
            .user_repo
            .find_by_id(&invite.created_by_user_id)
            .await
            .map(|user| user.email)
            .unwrap_or_else(|_| invite.created_by_user_id.to_string());

        templates.push(PendingApplicationAccessInviteTemplate {
            id: invite.id.to_string(),
            email: invite.email,
            expires_at: invite.expires_at.format("%Y-%m-%d %H:%M UTC").to_string(),
            created_by_email,
        });
    }

    Ok(templates)
}

fn render_application_access(
    status: axum::http::StatusCode,
    workspace_ctx: &WorkspaceContext,
    can_manage_application_access: bool,
    csrf_token: String,
    invite_email: String,
    pending_invites: Vec<PendingApplicationAccessInviteTemplate>,
    error: Option<String>,
    success: Option<String>,
) -> Response {
    render(
        status,
        ApplicationAccessTemplate {
            active: "/application-access",
            nip_prefix: None,
            user_email: workspace_ctx.user.email.clone(),
            current_workspace_name: workspace_ctx.workspace.display_name.clone(),
            can_manage_application_access,
            csrf_token,
            invite_email,
            pending_invites,
            error,
            success,
        },
    )
}

pub async fn page(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let can_manage_application_access = is_bootstrap_admin(&state, &workspace_ctx.user.email);
    let pending_invites = if can_manage_application_access {
        match load_pending_invites(&state).await {
            Ok(invites) => invites,
            Err(err) => {
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, err).into_response();
            }
        }
    } else {
        Vec::new()
    };

    render_application_access(
        if can_manage_application_access {
            axum::http::StatusCode::OK
        } else {
            axum::http::StatusCode::FORBIDDEN
        },
        &workspace_ctx,
        can_manage_application_access,
        csrf_token,
        String::new(),
        pending_invites,
        (!can_manage_application_access).then(|| ApplicationAccessError::Forbidden.to_string()),
        None,
    )
}

pub async fn create_invite(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
    CsrfForm(form): CsrfForm<CreateApplicationAccessInviteFormData>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    let email = normalize_email(&form.email);

    if let Err(err) = ensure_bootstrap_admin(&state, &workspace_ctx).await {
        return render_application_access(
            axum::http::StatusCode::FORBIDDEN,
            &workspace_ctx,
            false,
            csrf_token,
            email,
            Vec::new(),
            Some(err.to_string()),
            None,
        );
    }

    let pending_invites = match load_pending_invites(&state).await {
        Ok(invites) => invites,
        Err(err) => {
            return render_application_access(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &workspace_ctx,
                true,
                csrf_token,
                email,
                Vec::new(),
                Some(err),
                None,
            );
        }
    };

    if !is_valid_email(&email) {
        return render_application_access(
            axum::http::StatusCode::BAD_REQUEST,
            &workspace_ctx,
            true,
            csrf_token,
            email,
            pending_invites,
            Some(ApplicationAccessError::InvalidEmail.to_string()),
            None,
        );
    }

    if pending_invites.iter().any(|invite| invite.email == email) {
        return render_application_access(
            axum::http::StatusCode::CONFLICT,
            &workspace_ctx,
            true,
            csrf_token,
            email,
            pending_invites,
            Some(ApplicationAccessError::DuplicateInvite.to_string()),
            None,
        );
    }

    let raw_token = generate_invite_token();
    let invite = ApplicationAccessInvite {
        id: ApplicationAccessInviteId::new(),
        email: email.clone(),
        token_hash: hash_invite_token(&raw_token),
        expires_at: invite_expiration(),
        accepted_at: None,
        revoked_at: None,
        created_by_user_id: workspace_ctx.user.id.clone(),
        created_at: Utc::now(),
    };

    if let Err(err) = state.application_access_repo.create_invite(&invite).await {
        return render_application_access(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            true,
            csrf_token,
            email,
            pending_invites,
            Some(format!("Nie udało się utworzyć zaproszenia: {err}")),
            None,
        );
    }

    let invite_email = ApplicationAccessInviteEmail {
        recipient_email: invite.email.clone(),
        inviter_email: workspace_ctx.user.email.clone(),
        invite_url: invite_url(&state.public_base_url, &raw_token),
    };

    if let Err(err) =
        dispatch_application_access_invite(state.email_sender.clone(), invite_email).await
    {
        let _ = state
            .application_access_repo
            .revoke_invite(&invite.id)
            .await;
        let pending_invites = match load_pending_invites(&state).await {
            Ok(invites) => invites,
            Err(load_err) => {
                return render_application_access(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &workspace_ctx,
                    true,
                    csrf_token,
                    email,
                    Vec::new(),
                    Some(format!(
                        "Nie udało się wysłać zaproszenia email: {err}. Dodatkowo nie udało się odświeżyć listy zaproszeń: {load_err}"
                    )),
                    None,
                );
            }
        };
        return render_application_access(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            true,
            csrf_token,
            email,
            pending_invites,
            Some(format!("Nie udało się wysłać zaproszenia email: {err}")),
            None,
        );
    }

    let pending_invites = match load_pending_invites(&state).await {
        Ok(invites) => invites,
        Err(err) => {
            return render_application_access(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &workspace_ctx,
                true,
                csrf_token,
                String::new(),
                Vec::new(),
                Some(err),
                None,
            );
        }
    };
    render_application_access(
        axum::http::StatusCode::OK,
        &workspace_ctx,
        true,
        csrf_token,
        String::new(),
        pending_invites,
        None,
        Some(format!(
            "Wysłano email z dostępem do aplikacji dla {}.",
            invite.email
        )),
    )
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    workspace_ctx: WorkspaceContext,
    session: Session,
    Path(invite_id_raw): Path<String>,
    CsrfForm(_form): CsrfForm<RevokeApplicationAccessInviteFormData>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    if let Err(err) = ensure_bootstrap_admin(&state, &workspace_ctx).await {
        return render_application_access(
            axum::http::StatusCode::FORBIDDEN,
            &workspace_ctx,
            false,
            csrf_token,
            String::new(),
            Vec::new(),
            Some(err.to_string()),
            None,
        );
    }

    let pending_invites = match load_pending_invites(&state).await {
        Ok(invites) => invites,
        Err(err) => {
            return render_application_access(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &workspace_ctx,
                true,
                csrf_token,
                String::new(),
                Vec::new(),
                Some(err),
                None,
            );
        }
    };

    let invite_id: ApplicationAccessInviteId = match invite_id_raw.parse() {
        Ok(invite_id) => invite_id,
        Err(_) => {
            return render_application_access(
                axum::http::StatusCode::BAD_REQUEST,
                &workspace_ctx,
                true,
                csrf_token,
                String::new(),
                pending_invites,
                Some("Nieprawidłowy identyfikator zaproszenia.".to_string()),
                None,
            );
        }
    };

    let pending_for_check = match state.application_access_repo.list_pending_invites().await {
        Ok(invites) => invites,
        Err(err) => {
            return render_application_access(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &workspace_ctx,
                true,
                csrf_token,
                String::new(),
                pending_invites,
                Some(format!("Nie udało się sprawdzić zaproszenia: {err}")),
                None,
            );
        }
    };

    if !pending_for_check.iter().any(|invite| invite.id == invite_id) {
        return render_application_access(
            axum::http::StatusCode::NOT_FOUND,
            &workspace_ctx,
            true,
            csrf_token,
            String::new(),
            pending_invites,
            Some(ApplicationAccessError::InviteNotFound.to_string()),
            None,
        );
    }

    if let Err(err) = state
        .application_access_repo
        .revoke_invite(&invite_id)
        .await
    {
        return render_application_access(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &workspace_ctx,
            true,
            csrf_token,
            String::new(),
            pending_invites,
            Some(format!("Nie udało się odwołać zaproszenia: {err}")),
            None,
        );
    }

    let pending_invites = match load_pending_invites(&state).await {
        Ok(invites) => invites,
        Err(err) => {
            return render_application_access(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &workspace_ctx,
                true,
                csrf_token,
                String::new(),
                Vec::new(),
                Some(err),
                None,
            );
        }
    };
    render_application_access(
        axum::http::StatusCode::OK,
        &workspace_ctx,
        true,
        csrf_token,
        String::new(),
        pending_invites,
        None,
        Some("Zaproszenie do aplikacji zostało odwołane.".to_string()),
    )
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
    use crate::extractors::AuthUser;

    #[derive(Default)]
    struct RecordingEmailSender {
        workspace_sent: Mutex<Vec<WorkspaceInviteEmail>>,
        app_sent: Mutex<Vec<ApplicationAccessInviteEmail>>,
        fail: bool,
    }

    impl RecordingEmailSender {
        fn app_sent(&self) -> Vec<ApplicationAccessInviteEmail> {
            self.app_sent.lock().unwrap().clone()
        }
    }

    impl EmailSender for RecordingEmailSender {
        fn send_workspace_invite(
            &self,
            invite: WorkspaceInviteEmail,
        ) -> Result<(), EmailSendError> {
            self.workspace_sent.lock().unwrap().push(invite);
            Ok(())
        }

        fn send_application_access_invite(
            &self,
            invite: ApplicationAccessInviteEmail,
        ) -> Result<(), EmailSendError> {
            if self.fail {
                return Err(EmailSendError::Transport("smtp failed".to_string()));
            }
            self.app_sent.lock().unwrap().push(invite);
            Ok(())
        }
    }

    async fn test_state(email_sender: Arc<dyn EmailSender>) -> (AppState, std::path::PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "ksef-server-application-access-test-{}.db",
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

    async fn workspace_ctx(state: &AppState, user: &User) -> WorkspaceContext {
        let summary = state
            .workspace_repo
            .ensure_default_workspace(&user.id, &user.email)
            .await
            .unwrap();
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
    async fn bootstrap_admin_can_create_application_access_invite() {
        let sender = Arc::new(RecordingEmailSender::default());
        let (state, db_path) = test_state(sender.clone()).await;
        let admin = make_user("admin@example.com");
        state.user_repo.create(&admin).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &admin).await;

        let response = create_invite(
            State(state.clone()),
            workspace_ctx,
            session_with_csrf("csrf").await,
            CsrfForm(CreateApplicationAccessInviteFormData {
                email: "new.user@example.com".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("Wysłano email z dostępem do aplikacji"));
        let invites = state
            .application_access_repo
            .list_pending_invites()
            .await
            .unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].email, "new.user@example.com");
        let sent = sender.app_sent();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0]
                .invite_url
                .starts_with("https://app.example.test/register?app_access_invite=")
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn non_bootstrap_user_sees_rendered_forbidden_page() {
        let sender = Arc::new(RecordingEmailSender::default());
        let (state, db_path) = test_state(sender).await;
        let user = make_user("user@example.com");
        state.user_repo.create(&user).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &user).await;

        let response = page(
            State(state.clone()),
            workspace_ctx,
            session_with_csrf("csrf").await,
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_text(response).await;
        assert!(body.contains("Tylko bootstrap administrator"));
        assert!(body.contains("Workspace"));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn non_bootstrap_user_cannot_create_application_access_invite() {
        let sender = Arc::new(RecordingEmailSender::default());
        let (state, db_path) = test_state(sender).await;
        let user = make_user("user@example.com");
        state.user_repo.create(&user).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &user).await;

        let response = create_invite(
            State(state.clone()),
            workspace_ctx,
            session_with_csrf("csrf").await,
            CsrfForm(CreateApplicationAccessInviteFormData {
                email: "other@example.com".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let invites = state
            .application_access_repo
            .list_pending_invites()
            .await
            .unwrap();
        assert!(invites.is_empty());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn expired_application_access_invite_does_not_block_resend() {
        let sender = Arc::new(RecordingEmailSender::default());
        let (state, db_path) = test_state(sender.clone()).await;
        let admin = make_user("admin@example.com");
        state.user_repo.create(&admin).await.unwrap();
        let workspace_ctx = workspace_ctx(&state, &admin).await;

        state
            .application_access_repo
            .create_invite(&ApplicationAccessInvite {
                id: ApplicationAccessInviteId::new(),
                email: "expired.user@example.com".to_string(),
                token_hash: "expired-app-access-route".to_string(),
                expires_at: Utc::now() - chrono::Duration::days(1),
                accepted_at: None,
                revoked_at: None,
                created_by_user_id: admin.id.clone(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        let response = create_invite(
            State(state.clone()),
            workspace_ctx,
            session_with_csrf("csrf").await,
            CsrfForm(CreateApplicationAccessInviteFormData {
                email: "expired.user@example.com".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let invites = state
            .application_access_repo
            .list_pending_invites()
            .await
            .unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].email, "expired.user@example.com");

        let _ = std::fs::remove_file(db_path);
    }
}
