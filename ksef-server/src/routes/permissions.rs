use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use ksef_core::domain::nip::Nip;
use ksef_core::domain::permission::{
    PermissionGrantRequest, PermissionRecord, PermissionRevokeRequest, PermissionType,
};
use ksef_core::ports::ksef_permissions::PermissionQueryRequest;

use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/permissions.html")]
struct PermissionsTemplate {
    active: &'static str,
    records: Vec<PermissionRecord>,
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

fn empty_page(error: Option<String>, success: Option<String>) -> Response {
    render(PermissionsTemplate {
        active: "/permissions",
        records: Vec::new(),
        error,
        success,
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

pub async fn permissions_page() -> Response {
    empty_page(None, None)
}

pub async fn grant(State(state): State<AppState>, Form(form): Form<GrantFormData>) -> Response {
    let context_nip = match Nip::parse(&form.context_nip) {
        Ok(n) => n,
        Err(e) => return empty_page(Some(format!("NIP kontekstu: {e}")), None),
    };
    let authorized_nip = match Nip::parse(&form.authorized_nip) {
        Ok(n) => n,
        Err(e) => return empty_page(Some(format!("NIP uprawnionego: {e}")), None),
    };
    let permission: PermissionType = match form.permission.parse() {
        Ok(p) => p,
        Err(e) => return empty_page(Some(format!("Typ uprawnienia: {e}")), None),
    };

    let token = match state.session_service.ensure_token(&state.nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => return empty_page(Some(format!("Brak tokenu dostepu: {e}")), None),
    };

    let request = PermissionGrantRequest {
        context_nip,
        authorized_nip,
        permissions: vec![permission],
    };

    match state.permission_service.grant(&token, &request).await {
        Ok(()) => empty_page(None, Some("Uprawnienie nadane".to_string())),
        Err(e) => empty_page(
            Some(format!("Nadanie uprawnienia nie powiodlo sie: {e}")),
            None,
        ),
    }
}

pub async fn revoke(State(state): State<AppState>, Form(form): Form<RevokeFormData>) -> Response {
    let context_nip = match Nip::parse(&form.context_nip) {
        Ok(n) => n,
        Err(e) => return empty_page(Some(format!("NIP kontekstu: {e}")), None),
    };
    let authorized_nip = match Nip::parse(&form.authorized_nip) {
        Ok(n) => n,
        Err(e) => return empty_page(Some(format!("NIP uprawnionego: {e}")), None),
    };
    let permission: PermissionType = match form.permission.parse() {
        Ok(p) => p,
        Err(e) => return empty_page(Some(format!("Typ uprawnienia: {e}")), None),
    };

    let token = match state.session_service.ensure_token(&state.nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => return empty_page(Some(format!("Brak tokenu dostepu: {e}")), None),
    };

    let request = PermissionRevokeRequest {
        context_nip,
        authorized_nip,
        permissions: vec![permission],
    };

    match state.permission_service.revoke(&token, &request).await {
        Ok(()) => empty_page(None, Some("Uprawnienie odebrane".to_string())),
        Err(e) => empty_page(
            Some(format!("Odebranie uprawnienia nie powiodlo sie: {e}")),
            None,
        ),
    }
}

pub async fn query(State(state): State<AppState>, Form(form): Form<QueryFormData>) -> Response {
    let context_nip = match Nip::parse(&form.context_nip) {
        Ok(n) => n,
        Err(e) => return empty_page(Some(format!("NIP kontekstu: {e}")), None),
    };

    let token = match state.session_service.ensure_token(&state.nip).await {
        Ok(tp) => tp.access_token,
        Err(e) => return empty_page(Some(format!("Brak tokenu dostepu: {e}")), None),
    };

    let request = PermissionQueryRequest {
        context_nip,
        authorized_nip: None,
        permission: None,
    };

    match state.permission_service.query(&token, &request).await {
        Ok(records) => render(PermissionsTemplate {
            active: "/permissions",
            records,
            error: None,
            success: None,
        }),
        Err(e) => empty_page(
            Some(format!("Zapytanie o uprawnienia nie powiodlo sie: {e}")),
            None,
        ),
    }
}
