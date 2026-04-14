use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use ksef_core::domain::permission::PermissionType;
use ksef_core::domain::token_mgmt::ManagedToken;
use ksef_core::ports::ksef_tokens::{TokenGenerateRequest, TokenQueryRequest};

use crate::extractors::NipContext;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/tokens.html")]
struct TokensTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    tokens: Vec<ManagedToken>,
    error: Option<String>,
    success: Option<String>,
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
) -> Response {
    render(TokensTemplate {
        active: "/tokens",
        nip_prefix: Some(nip_prefix),
        user_email,
        tokens: Vec::new(),
        error,
        success,
    })
}

#[derive(Deserialize)]
pub struct GenerateFormData {
    /// Comma-separated permission names from the hidden form field.
    #[serde(default)]
    pub permissions: String,
    pub description: Option<String>,
}

pub async fn tokens_page(State(state): State<AppState>, nip_ctx: NipContext) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_email = nip_ctx.user.email;

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Brak tokenu dostepu: {e}")),
                None,
            );
        }
    };

    let request = TokenQueryRequest {
        status: None,
        limit: Some(50),
        offset: None,
    };

    match state.token_mgmt_service.query(&token, &request).await {
        Ok(response) => render(TokensTemplate {
            active: "/tokens",
            nip_prefix: Some(nip_str),
            user_email,
            tokens: response.items,
            error: None,
            success: None,
        }),
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Pobieranie tokenow nie powiodlo sie: {e}")),
            None,
        ),
    }
}

pub async fn generate(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    Form(form): Form<GenerateFormData>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_email = nip_ctx.user.email;

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
        );
    }
    let permissions: Result<Vec<PermissionType>, _> =
        raw_permissions.iter().map(|s| s.parse()).collect();
    let permissions = match permissions {
        Ok(p) => p,
        Err(e) => {
            return empty_page(nip_str, user_email, Some(format!("Uprawnienia: {e}")), None);
        }
    };

    let token = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Brak tokenu dostepu: {e}")),
                None,
            );
        }
    };

    let request = TokenGenerateRequest {
        permissions,
        description: form.description.filter(|s| !s.trim().is_empty()),
        valid_to: None,
    };

    match state.token_mgmt_service.generate(&token, &request).await {
        Ok(generated) => empty_page(
            nip_str,
            user_email,
            None,
            Some(format!("Token wygenerowany: {}", generated.id)),
        ),
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Generowanie tokenu nie powiodlo sie: {e}")),
            None,
        ),
    }
}

pub async fn revoke(
    State(state): State<AppState>,
    nip_ctx: NipContext,
    Path((_nip, token_id)): Path<(String, String)>,
) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let user_email = nip_ctx.user.email;

    let access = match state.session_service.ensure_token(nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => {
            return empty_page(
                nip_str,
                user_email,
                Some(format!("Brak tokenu dostepu: {e}")),
                None,
            );
        }
    };

    match state.token_mgmt_service.revoke(&access, &token_id).await {
        Ok(()) => Redirect::to(&format!("/accounts/{nip_str}/tokens")).into_response(),
        Err(e) => empty_page(
            nip_str,
            user_email,
            Some(format!("Unieaznienie tokenu nie powiodlo sie: {e}")),
            None,
        ),
    }
}
