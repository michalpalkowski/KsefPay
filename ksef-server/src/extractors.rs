use axum::Form;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tower_sessions::Session;

use ksef_core::domain::account_scope::AccountScope;
use ksef_core::domain::nip::Nip;
use ksef_core::domain::nip_account::NipAccount;
use ksef_core::domain::user::UserId;
use ksef_core::domain::workspace::{Workspace, WorkspaceId, WorkspaceMembership};
use ksef_core::error::RepositoryError;

use crate::csrf::CSRF_SESSION_KEY;
use crate::state::AppState;

pub const CURRENT_WORKSPACE_SESSION_KEY: &str = "workspace_id";

/// Extractor: authenticated user from tower-sessions.
///
/// Reads `user_id` from the session. If absent or the user no longer exists,
/// redirects to `/login`.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: UserId,
    pub email: String,
}

impl IntoResponse for AuthUserRejection {
    fn into_response(self) -> Response {
        match self {
            Self::NotLoggedIn => Redirect::to("/login").into_response(),
            Self::InternalError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}

#[derive(Debug, Error)]
pub enum AuthUserRejection {
    #[error("user is not logged in")]
    NotLoggedIn,
    #[error("authentication session error: {0}")]
    InternalError(String),
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthUserRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|e| AuthUserRejection::InternalError(format!("session error: {e:?}")))?;

        let user_id_str: String = session
            .get("user_id")
            .await
            .map_err(|e| AuthUserRejection::InternalError(format!("session read error: {e}")))?
            .ok_or(AuthUserRejection::NotLoggedIn)?;

        let user_id: UserId = user_id_str
            .parse()
            .map_err(|_| AuthUserRejection::NotLoggedIn)?;

        let Ok(user) = state.user_repo.find_by_id(&user_id).await else {
            // User not found -- clear session and redirect to login
            let _ = session.delete().await;
            return Err(AuthUserRejection::NotLoggedIn);
        };

        Ok(Self {
            id: user.id,
            email: user.email,
        })
    }
}

/// Extractor: NIP-scoped context from URL path `{nip}` parameter.
///
/// Requires an authenticated user (via [`AuthUser`]) and verifies that the
/// user has access to the NIP account identified by the path parameter.
#[derive(Debug, Clone)]
pub struct NipContext {
    pub user: AuthUser,
    pub membership: WorkspaceMembership,
    /// Account details for display (NIP string, display name, auth method, etc.).
    pub account: NipAccount,
    /// Proof-of-authorization — pass this to per-account service and repository calls.
    pub scope: AccountScope,
}

#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub user: AuthUser,
    pub workspace: Workspace,
    pub membership: WorkspaceMembership,
}

#[derive(Debug, Error)]
#[error("authentication rejected")]
pub enum WorkspaceContextRejection {
    #[error(transparent)]
    Auth(#[from] AuthUserRejection),
    #[error("user has no active workspace")]
    NoWorkspace,
    #[error("session error: {0}")]
    Session(String),
    #[error("workspace repository error: {0}")]
    Repository(#[from] RepositoryError),
}

impl IntoResponse for WorkspaceContextRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(inner) => inner.into_response(),
            Self::NoWorkspace => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Brak aktywnego workspace dla użytkownika",
            )
                .into_response(),
            Self::Session(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
            Self::Repository(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Błąd workspace: {err}"),
            )
                .into_response(),
        }
    }
}

impl FromRequestParts<AppState> for WorkspaceContext {
    type Rejection = WorkspaceContextRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state)
            .await
            .map_err(WorkspaceContextRejection::from)?;
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|e| WorkspaceContextRejection::Session(format!("session error: {e:?}")))?;

        let workspaces = state.workspace_repo.list_for_user(&user.id).await?;

        let selected_workspace = session
            .get::<String>(CURRENT_WORKSPACE_SESSION_KEY)
            .await
            .map_err(|e| WorkspaceContextRejection::Session(format!("session read error: {e}")))?
            .and_then(|value| value.parse::<WorkspaceId>().ok());

        let current = selected_workspace
            .as_ref()
            .and_then(|workspace_id| {
                workspaces
                    .iter()
                    .find(|summary| &summary.workspace.id == workspace_id)
                    .cloned()
            })
            .or_else(|| workspaces.first().cloned())
            .ok_or(WorkspaceContextRejection::NoWorkspace)?;

        if selected_workspace.as_ref() != Some(&current.workspace.id) {
            session
                .insert(
                    CURRENT_WORKSPACE_SESSION_KEY,
                    current.workspace.id.to_string(),
                )
                .await
                .map_err(|e| {
                    WorkspaceContextRejection::Session(format!("session write error: {e}"))
                })?;
        }

        Ok(Self {
            user,
            workspace: current.workspace,
            membership: current.membership,
        })
    }
}

#[derive(Debug, Error)]
#[error("NIP context rejected")]
pub enum NipContextRejection {
    #[error(transparent)]
    Workspace(#[from] WorkspaceContextRejection),
    #[error("forbidden NIP access")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("workspace repository error: {0}")]
    Repository(#[from] RepositoryError),
}

impl IntoResponse for NipContextRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Workspace(inner) => inner.into_response(),
            Self::Forbidden => {
                (StatusCode::FORBIDDEN, "Brak dostępu do tego konta NIP").into_response()
            }
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::Repository(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Błąd repozytorium workspace: {err}"),
            )
                .into_response(),
        }
    }
}

impl FromRequestParts<AppState> for NipContext {
    type Rejection = NipContextRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let workspace_ctx = WorkspaceContext::from_request_parts(parts, state)
            .await
            .map_err(NipContextRejection::from)?;

        let nip_raw: String = parts
            .extensions
            .get::<axum::extract::Path<std::collections::HashMap<String, String>>>()
            .and_then(|p| p.get("nip").cloned())
            .or_else(|| {
                // Fall back to parsing from the URI path
                extract_nip_from_uri(&parts.uri)
            })
            .ok_or_else(|| {
                NipContextRejection::BadRequest("brak parametru NIP w ścieżce".to_string())
            })?;

        let nip = Nip::parse(&nip_raw)
            .map_err(|e| NipContextRejection::BadRequest(format!("nieprawidłowy NIP: {e}")))?;

        let (account, scope, _membership) = state
            .workspace_repo
            .find_user_account_in_workspace(
                &workspace_ctx.workspace.id,
                &workspace_ctx.user.id,
                &nip,
            )
            .await
            .map_err(NipContextRejection::from)?
            .ok_or(NipContextRejection::Forbidden)?;

        Ok(Self {
            user: workspace_ctx.user,
            membership: workspace_ctx.membership,
            account,
            scope,
        })
    }
}

/// Extract the NIP segment from a URI path like `/accounts/{nip}/...`.
fn extract_nip_from_uri(uri: &axum::http::Uri) -> Option<String> {
    let path = uri.path();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Expected: ["accounts", "{nip}", ...]
    if segments.len() >= 2 && segments[0] == "accounts" {
        Some(segments[1].to_string())
    } else {
        None
    }
}

/// Form extractor with built-in CSRF validation.
///
/// Expects a `_csrf` field in URL-encoded form body and compares it with
/// the token stored in the session.
pub struct CsrfForm<T>(pub T);

#[derive(Deserialize)]
struct CsrfPayload<T> {
    #[serde(rename = "_csrf")]
    csrf: String,
    #[serde(flatten)]
    inner: T,
}

#[derive(Debug, Error)]
pub enum CsrfFormRejection {
    #[error("invalid csrf token")]
    Forbidden,
    #[error("invalid form data: {0}")]
    BadRequest(String),
    #[error("csrf/session error: {0}")]
    InternalError(String),
}

impl IntoResponse for CsrfFormRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "Żądanie odrzucone: nieprawidłowy token CSRF",
            )
                .into_response(),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::InternalError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}

impl<T> FromRequest<AppState> for CsrfForm<T>
where
    T: DeserializeOwned + Send,
{
    type Rejection = CsrfFormRejection;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();

        let session = Session::from_request_parts(&mut parts, state)
            .await
            .map_err(|e| CsrfFormRejection::InternalError(format!("session error: {e:?}")))?;

        let expected = session
            .get::<String>(CSRF_SESSION_KEY)
            .await
            .map_err(|e| CsrfFormRejection::InternalError(format!("session read error: {e}")))?
            .ok_or(CsrfFormRejection::Forbidden)?;

        let req = Request::from_parts(parts, body);
        let Form(payload) = Form::<CsrfPayload<T>>::from_request(req, state)
            .await
            .map_err(|e| CsrfFormRejection::BadRequest(format!("invalid form data: {e}")))?;

        if payload.csrf != expected {
            return Err(CsrfFormRejection::Forbidden);
        }

        Ok(Self(payload.inner))
    }
}
