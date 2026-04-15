use askama::Template;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use tower_sessions::Session;

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::nip::Nip;
use ksef_core::domain::permission::{
    PermissionGrantRequest, PermissionRecord, PermissionRevokeRequest, PermissionType,
};
use ksef_core::ports::ksef_permissions::PermissionQueryRequest;

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::{CsrfForm, NipContext};
use crate::request_meta::client_ip;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/permissions.html")]
struct PermissionsTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    records: Vec<PermissionRecord>,
    error: Option<String>,
    success: Option<String>,
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

fn empty_page(
    nip_prefix: String,
    user_email: String,
    error: Option<String>,
    success: Option<String>,
    csrf_token: String,
) -> Response {
    render(PermissionsTemplate {
        active: "/permissions",
        nip_prefix: Some(nip_prefix),
        user_email,
        records: Vec::new(),
        error,
        success,
        csrf_token,
    })
}

#[derive(Deserialize)]
pub struct GrantFormData {
    pub context_nip: String,
    pub authorized_nip: String,
    pub permission: String,
}

#[derive(Deserialize)]
pub struct RevokeFormData {
    pub context_nip: String,
    pub authorized_nip: String,
    pub permission: String,
}

#[derive(Deserialize)]
pub struct QueryFormData {
    pub context_nip: String,
}

pub async fn permissions_page(nip_ctx: NipContext, session: Session) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    empty_page(
        nip_ctx.account.nip.to_string(),
        nip_ctx.user.email,
        None,
        None,
        csrf_token,
    )
}

pub async fn grant(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<GrantFormData>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let context_nip = match Nip::parse(&form.context_nip) {
        Ok(n) => n,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("NIP kontekstu: {e}")),
                None,
                csrf_token,
            );
        }
    };
    let authorized_nip = match Nip::parse(&form.authorized_nip) {
        Ok(n) => n,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("NIP uprawnionego: {e}")),
                None,
                csrf_token,
            );
        }
    };
    let permission: PermissionType = match form.permission.parse() {
        Ok(p) => p,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Typ uprawnienia: {e}")),
                None,
                csrf_token,
            );
        }
    };

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Brak tokenu dostępu: {e}")),
                None,
                csrf_token,
            );
        }
    };

    let request = PermissionGrantRequest {
        context_nip,
        authorized_nip,
        permissions: vec![permission],
    };

    match state.permission_service.grant(&token, &request).await {
        Ok(()) => {
            log_audit_action(
                &state,
                &user_id,
                &user_email,
                Some(nip),
                AuditAction::GrantPermission,
                Some(format!(
                    "context_nip={},authorized_nip={},permission={permission}",
                    request.context_nip, request.authorized_nip
                )),
                client_ip(&headers),
            )
            .await;

            empty_page(
                nip_str,
                user_email,
                None,
                Some("Uprawnienie nadane".to_string()),
                csrf_token,
            )
        }
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Nadanie uprawnienia nie powiodło się: {e}")),
            None,
            csrf_token,
        ),
    }
}

pub async fn revoke(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<RevokeFormData>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let context_nip = match Nip::parse(&form.context_nip) {
        Ok(n) => n,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("NIP kontekstu: {e}")),
                None,
                csrf_token,
            );
        }
    };
    let authorized_nip = match Nip::parse(&form.authorized_nip) {
        Ok(n) => n,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("NIP uprawnionego: {e}")),
                None,
                csrf_token,
            );
        }
    };
    let permission: PermissionType = match form.permission.parse() {
        Ok(p) => p,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Typ uprawnienia: {e}")),
                None,
                csrf_token,
            );
        }
    };

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Brak tokenu dostępu: {e}")),
                None,
                csrf_token,
            );
        }
    };

    let request = PermissionRevokeRequest {
        context_nip,
        authorized_nip,
        permissions: vec![permission],
    };

    match state.permission_service.revoke(&token, &request).await {
        Ok(()) => {
            log_audit_action(
                &state,
                &user_id,
                &user_email,
                Some(nip),
                AuditAction::RevokePermission,
                Some(format!(
                    "context_nip={},authorized_nip={},permission={permission}",
                    request.context_nip, request.authorized_nip
                )),
                client_ip(&headers),
            )
            .await;

            empty_page(
                nip_str,
                user_email,
                None,
                Some("Uprawnienie odebrane".to_string()),
                csrf_token,
            )
        }
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Odebranie uprawnienia nie powiodło się: {e}")),
            None,
            csrf_token,
        ),
    }
}

pub async fn query(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    session: Session,
    CsrfForm(form): CsrfForm<QueryFormData>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let context_nip = match Nip::parse(&form.context_nip) {
        Ok(n) => n,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("NIP kontekstu: {e}")),
                None,
                csrf_token,
            );
        }
    };

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Brak tokenu dostępu: {e}")),
                None,
                csrf_token,
            );
        }
    };

    let request = PermissionQueryRequest {
        context_nip,
        authorized_nip: None,
        permission: None,
    };

    match state.permission_service.query(&token, &request).await {
        Ok(records) => render(PermissionsTemplate {
            active: "/permissions",
            nip_prefix: Some(nip_str),
            user_email,
            records,
            error: None,
            success: None,
            csrf_token,
        }),
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Zapytanie o uprawnienia nie powiodło się: {e}")),
            None,
            csrf_token,
        ),
    }
}
