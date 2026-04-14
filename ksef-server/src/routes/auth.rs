use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use tower_sessions::Session;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use ksef_core::domain::user::{User, UserId};

use crate::state::AppState;

// --- Templates ---

#[derive(Template)]
#[template(path = "pages/login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/register.html")]
struct RegisterTemplate {
    error: Option<String>,
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

// --- Handlers ---

pub async fn login_page(session: Session) -> Response {
    // If already logged in, redirect to accounts
    if let Ok(Some(_)) = session.get::<String>("user_id").await {
        return Redirect::to("/accounts").into_response();
    }
    render(LoginTemplate { error: None })
}

pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<LoginFormData>,
) -> Response {
    let email = form.email.trim().to_lowercase();
    if email.is_empty() || form.password.is_empty() {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            LoginTemplate {
                error: Some("Email i haslo sa wymagane".to_string()),
            },
        );
    }

    let user = match state.user_repo.find_by_email(&email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return render_with_status(
                StatusCode::UNAUTHORIZED,
                LoginTemplate {
                    error: Some("Nieprawidlowy email lub haslo".to_string()),
                },
            );
        }
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                LoginTemplate {
                    error: Some(format!("Blad serwera: {e}")),
                },
            );
        }
    };

    let Ok(parsed_hash) = PasswordHash::new(&user.password_hash) else {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            LoginTemplate {
                error: Some("Blad weryfikacji hasla".to_string()),
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
                error: Some("Nieprawidlowy email lub haslo".to_string()),
            },
        );
    }

    // Create session
    if let Err(e) = session
        .insert("user_id", user.id.to_string())
        .await
    {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            LoginTemplate {
                error: Some(format!("Blad sesji: {e}")),
            },
        );
    }

    Redirect::to("/accounts").into_response()
}

pub async fn register_page(session: Session) -> Response {
    // If already logged in, redirect to accounts
    if let Ok(Some(_)) = session.get::<String>("user_id").await {
        return Redirect::to("/accounts").into_response();
    }
    render(RegisterTemplate { error: None })
}

pub async fn register(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<RegisterFormData>,
) -> Response {
    let email = form.email.trim().to_lowercase();

    // Validate input
    if email.is_empty() || form.password.is_empty() {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Email i haslo sa wymagane".to_string()),
            },
        );
    }

    if !email.contains('@') {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Nieprawidlowy adres email".to_string()),
            },
        );
    }

    if form.password.len() < 8 {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Haslo musi miec co najmniej 8 znakow".to_string()),
            },
        );
    }

    if form.password != form.password_confirm {
        return render_with_status(
            StatusCode::BAD_REQUEST,
            RegisterTemplate {
                error: Some("Hasla nie sa zgodne".to_string()),
            },
        );
    }

    // Check if user already exists
    match state.user_repo.find_by_email(&email).await {
        Ok(Some(_)) => {
            return render_with_status(
                StatusCode::CONFLICT,
                RegisterTemplate {
                    error: Some("Konto z tym adresem email juz istnieje".to_string()),
                },
            );
        }
        Ok(None) => {}
        Err(e) => {
            return render_with_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                RegisterTemplate {
                    error: Some(format!("Blad serwera: {e}")),
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
                    error: Some(format!("Blad hashowania hasla: {e}")),
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
                error: Some(format!("Nie udalo sie utworzyc konta: {e}")),
            },
        );
    }

    // Auto-login
    if let Err(e) = session.insert("user_id", user.id.to_string()).await {
        return render_with_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            RegisterTemplate {
                error: Some(format!("Blad sesji: {e}")),
            },
        );
    }

    Redirect::to("/accounts").into_response()
}

pub async fn logout(session: Session) -> Response {
    let _ = session.delete().await;
    Redirect::to("/login").into_response()
}
