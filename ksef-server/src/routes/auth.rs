use askama::Template;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use tower_sessions::Session;

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::user::{User, UserId};

use crate::audit_log::log_action as log_audit_action;
use crate::csrf::ensure_csrf_token;
use crate::extractors::CsrfForm;
use crate::request_meta::client_ip;
use crate::state::AppState;

// --- Templates ---

#[derive(Template)]
#[template(path = "pages/login.html")]
struct LoginTemplate {
    error: Option<String>,
    email: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "pages/register.html")]
struct RegisterTemplate {
    error: Option<String>,
    email: String,
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

// --- Form data ---

#[derive(Deserialize)]
pub struct LoginFormData {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct RegisterFormData {
    pub email: String,
    pub password: String,
    pub password_confirm: String,
}

// --- Validation ---

fn is_valid_email(email: &str) -> bool {
    // Basic structural check: something@something.something
    // No external regex crate needed — just enforce minimum structure.
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let (local, domain) = (parts[0], parts[1]);
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    // Domain must have at least one dot and no consecutive dots
    if !domain.contains('.') || domain.contains("..") {
        return false;
    }
    // Domain must not start/end with dot or hyphen
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

// --- Handlers ---

pub async fn login_page(session: Session) -> Response {
    // If already logged in, redirect to accounts
    if let Ok(Some(_)) = session.get::<String>("user_id").await {
        return Redirect::to("/accounts").into_response();
    }
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    render(LoginTemplate {
        error: None,
        email: String::new(),
        csrf_token,
    })
}

pub async fn login(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    CsrfForm(form): CsrfForm<LoginFormData>,
) -> Response {
    let email = form.email.trim().to_lowercase();
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    if let Some(retry_after) = auth_rate_limit_retry_after(&state, &headers) {
        return with_retry_after(
            render_with_status(
                StatusCode::TOO_MANY_REQUESTS,
                LoginTemplate {
                    error: Some(format!(
                        "Zbyt wiele prób logowania. Spróbuj ponownie za {retry_after} sekund."
                    )),
                    email,
                    csrf_token,
                },
            ),
            retry_after,
        );
    }

    if email.is_empty() || form.password.is_empty() {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            LoginTemplate {
                error: Some("Email i hasło są wymagane".to_string()),
                email,
                csrf_token,
            },
        );
    }

    let user = match state.user_repo.find_by_email(&email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return render_with_status(
                StatusCode::UNAUTHORIZED,
                LoginTemplate {
                    error: Some("Nieprawidłowy email lub hasło".to_string()),
                    email,
                    csrf_token,
                },
            );
        }
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                LoginTemplate {
                    error: Some(format!("Błąd serwera: {e}")),
                    email,
                    csrf_token,
                },
            );
        }
    };

    let Ok(parsed_hash) = PasswordHash::new(&user.password_hash) else {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            LoginTemplate {
                error: Some("Błąd weryfikacji hasła".to_string()),
                email,
                csrf_token,
            },
        );
    };

    if Argon2::default()
        .verify_password(form.password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return render_with_status(
            StatusCode::UNAUTHORIZED,
            LoginTemplate {
                error: Some("Nieprawidłowy email lub hasło".to_string()),
                email,
                csrf_token,
            },
        );
    }

    // Create session
    if let Err(e) = session.insert("user_id", user.id.to_string()).await {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            LoginTemplate {
                error: Some(format!("Błąd sesji: {e}")),
                email,
                csrf_token,
            },
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

pub async fn register_page(session: Session) -> Response {
    // If already logged in, redirect to accounts
    if let Ok(Some(_)) = session.get::<String>("user_id").await {
        return Redirect::to("/accounts").into_response();
    }
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();
    render(RegisterTemplate {
        error: None,
        email: String::new(),
        csrf_token,
    })
}

pub async fn register(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    CsrfForm(form): CsrfForm<RegisterFormData>,
) -> Response {
    let email = form.email.trim().to_lowercase();
    let csrf_token = ensure_csrf_token(&session).await.unwrap_or_default();

    if let Some(retry_after) = auth_rate_limit_retry_after(&state, &headers) {
        return with_retry_after(
            render_with_status(
                StatusCode::TOO_MANY_REQUESTS,
                RegisterTemplate {
                    error: Some(format!(
                        "Zbyt wiele prób rejestracji. Spróbuj ponownie za {retry_after} sekund."
                    )),
                    email,
                    csrf_token,
                },
            ),
            retry_after,
        );
    }

    // Validate input
    if email.is_empty() || form.password.is_empty() {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Email i hasło są wymagane".to_string()),
                email,
                csrf_token,
            },
        );
    }

    if !is_valid_email(&email) {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Nieprawidłowy adres email".to_string()),
                email,
                csrf_token,
            },
        );
    }

    if let Err(msg) = validate_password_strength(&form.password) {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some(msg),
                email,
                csrf_token,
            },
        );
    }

    if form.password != form.password_confirm {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Hasła nie są zgodne".to_string()),
                email,
                csrf_token,
            },
        );
    }

    // Check if user already exists
    match state.user_repo.find_by_email(&email).await {
        Ok(Some(_)) => {
            return render_with_status(
                StatusCode::CONFLICT,
                RegisterTemplate {
                    error: Some("Konto z tym adresem email już istnieje".to_string()),
                    email,
                    csrf_token,
                },
            );
        }
        Ok(None) => {}
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                RegisterTemplate {
                    error: Some(format!("Błąd serwera: {e}")),
                    email,
                    csrf_token,
                },
            );
        }
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = match Argon2::default().hash_password(form.password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                RegisterTemplate {
                    error: Some(format!("Błąd hashowania hasła: {e}")),
                    email,
                    csrf_token,
                },
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
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            RegisterTemplate {
                error: Some(format!("Nie udało się utworzyć konta: {e}")),
                email,
                csrf_token,
            },
        );
    }

    // Auto-login
    if let Err(e) = session.insert("user_id", user.id.to_string()).await {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            RegisterTemplate {
                error: Some(format!("Błąd sesji: {e}")),
                email,
                csrf_token,
            },
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
