use askama::Template;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use tower_sessions::Session;

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use ksef_core::domain::audit::AuditAction;

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::{AuthUser, CsrfForm};
use crate::request_meta::client_ip;
use crate::state::AppState;

use super::auth::validate_password_strength;

#[derive(Template)]
#[template(path = "pages/profile.html")]
struct ProfileTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
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

fn page(
    email: &str,
    error: Option<String>,
    success: Option<String>,
    csrf_token: String,
) -> ProfileTemplate {
    ProfileTemplate {
        active: "/profile",
        nip_prefix: None,
        user_email: email.to_string(),
        error,
        success,
        csrf_token,
    }
}

#[derive(Deserialize)]
pub struct ChangePasswordForm {
    pub current_password: String,
    pub new_password: String,
    pub new_password_confirm: String,
}

pub async fn profile_page(auth: AuthUser, session: Session) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    render(page(&auth.email, None, None, csrf_token))
}

pub async fn change_password(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    session: Session,
    CsrfForm(form): CsrfForm<ChangePasswordForm>,
) -> Response {
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    // Load user to verify current password
    let user = match state.user_repo.find_by_id(&auth.id).await {
        Ok(u) => u,
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                page(
                    &auth.email,
                    Some(format!("Błąd serwera: {e}")),
                    None,
                    csrf_token,
                ),
            );
        }
    };

    // Verify current password
    let Ok(parsed_hash) = PasswordHash::new(&user.password_hash) else {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            page(
                &auth.email,
                Some("Błąd weryfikacji hasła".to_string()),
                None,
                csrf_token,
            ),
        );
    };

    if Argon2::default()
        .verify_password(form.current_password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            page(
                &auth.email,
                Some("Obecne hasło jest nieprawidłowe".to_string()),
                None,
                csrf_token,
            ),
        );
    }

    // Validate new password
    if let Err(msg) = validate_password_strength(&form.new_password) {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            page(&auth.email, Some(msg), None, csrf_token),
        );
    }

    if form.new_password != form.new_password_confirm {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            page(
                &auth.email,
                Some("Nowe hasła nie są zgodne".to_string()),
                None,
                csrf_token,
            ),
        );
    }

    // Hash new password
    let salt = SaltString::generate(&mut OsRng);
    let new_hash = match Argon2::default().hash_password(form.new_password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                page(
                    &auth.email,
                    Some(format!("Błąd hashowania: {e}")),
                    None,
                    csrf_token,
                ),
            );
        }
    };

    // Update in DB
    let mut updated_user = user;
    updated_user.password_hash = new_hash;
    updated_user.updated_at = chrono::Utc::now();
    if let Err(e) = state.user_repo.update_password(&updated_user).await {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            page(
                &auth.email,
                Some(format!("Nie udało się zmienić hasła: {e}")),
                None,
                csrf_token,
            ),
        );
    }

    log_audit_action(
        &state,
        &auth.id,
        &auth.email,
        None,
        AuditAction::ChangePassword,
        None,
        client_ip(&headers),
    )
    .await;

    render(page(
        &auth.email,
        None,
        Some("Hasło zostało zmienione".to_string()),
        csrf_token,
    ))
}
