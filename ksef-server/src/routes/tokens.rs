use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use tower_sessions::Session;
use uuid::Uuid;

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::permission::PermissionType;
use ksef_core::domain::token_mgmt::LocalToken;
use ksef_core::ports::ksef_tokens::TokenGenerateRequest;

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::{CsrfForm, NipContext};
use crate::request_meta::client_ip;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/tokens.html")]
struct TokensTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    tokens: Vec<LocalToken>,
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
    render(TokensTemplate {
        active: "/tokens",
        nip_prefix: Some(nip_prefix),
        user_email,
        tokens: Vec::new(),
        error,
        success,
        csrf_token,
    })
}

#[derive(Deserialize)]
pub struct GenerateFormData {
    /// Comma-separated permission names from the hidden form field.
    #[serde(default)]
    pub permissions: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct RevokeTokenForm {}

pub async fn tokens_page(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    session: Session,
) -> Response {
    let nip_str = nip_ctx.account.nip.to_string();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let tokens = match state
        .local_token_repo
        .list_by_account_for_user(&nip_ctx.account.id, &user_id)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Pobieranie tokenów nie powiodło się: {e}")),
                None,
                csrf_token,
            );
        }
    };

    render(TokensTemplate {
        active: "/tokens",
        nip_prefix: Some(nip_str),
        user_email,
        tokens,
        error: None,
        success: None,
        csrf_token,
    })
}

pub async fn generate(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<GenerateFormData>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let raw_permissions: Vec<&str> = form
        .permissions
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if raw_permissions.is_empty() {
        return empty_page(
            nip_str,
            user_email,
            Some("Wymagane co najmniej jedno uprawnienie".to_string()),
            None,
            csrf_token,
        );
    }
    let permissions: Result<Vec<PermissionType>, _> =
        raw_permissions.iter().map(|s| s.parse()).collect();
    let permissions = match permissions {
        Ok(p) => p,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Uprawnienia: {e}")),
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

    let description = form.description.filter(|s| !s.trim().is_empty());
    let request = TokenGenerateRequest {
        permissions: permissions.clone(),
        description: description.clone(),
        valid_to: None,
    };

    match state.token_mgmt_service.generate(&token, &request).await {
        Ok(generated) => {
            log_audit_action(
                &state,
                &user_id,
                &user_email,
                Some(nip),
                AuditAction::GenerateToken,
                Some(format!("token_id={}", generated.id)),
                client_ip(&headers),
            )
            .await;

            // Persist locally so the page can filter by NIP + user.
            let local = LocalToken {
                id: Uuid::new_v4(),
                nip_account_id: nip_ctx.account.id.clone(),
                user_id: user_id.clone(),
                ksef_token_id: generated.id.clone(),
                permissions,
                description,
                created_at: generated.created_at,
                revoked_at: None,
            };
            if let Err(e) = state.local_token_repo.save(&local).await {
                tracing::warn!(
                    token_id = %generated.id,
                    "failed to persist local token entry: {e}"
                );
                return empty_page(
                    nip_str,
                    user_email,
                    Some(format!(
                        "Token został wygenerowany w KSeF (ID: {}), ale zapis lokalny nie powiódł się: {e}",
                        generated.id
                    )),
                    None,
                    csrf_token,
                );
            }

            Redirect::to(&format!("/accounts/{nip_str}/tokens")).into_response()
        }
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Generowanie tokenu nie powiodło się: {e}")),
            None,
            csrf_token,
        ),
    }
}

pub async fn revoke(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    session: Session,
    Path((_nip, token_id)): Path<(String, String)>,
    headers: HeaderMap,
    CsrfForm(_form): CsrfForm<RevokeTokenForm>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_id = nip_ctx.user.id.clone();
    let user_email = nip_ctx.user.email;
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    let access = match state.session_service.ensure_token(nip).await {
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

    match state.token_mgmt_service.revoke(&access, &token_id).await {
        Ok(()) => {
            log_audit_action(
                &state,
                &user_id,
                &user_email,
                Some(nip),
                AuditAction::RevokeToken,
                Some(format!("token_id={token_id}")),
                client_ip(&headers),
            )
            .await;

            if let Err(e) = state
                .local_token_repo
                .mark_revoked(&token_id, &nip_ctx.account.id)
                .await
            {
                tracing::warn!(token_id = %token_id, "failed to mark local token revoked: {e}");
            }

            Redirect::to(&format!("/accounts/{nip_str}/tokens")).into_response()
        }
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Unieważnienie tokenu nie powiodło się: {e}")),
            None,
            csrf_token,
        ),
    }
}
