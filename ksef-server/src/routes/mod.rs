use axum::Router;
use axum::extract::Request;
use axum::response::Redirect;
use axum::routing::{get, post};

pub mod accounts;
pub mod auth;
mod dashboard;
mod export;
mod fetch;
mod health;
mod invoices;
mod permissions;
mod sessions;
mod tokens;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Public routes (no auth required)
        .route("/login", get(auth::login_page).post(auth::login))
        .route("/register", get(auth::register_page).post(auth::register))
        .route("/logout", get(auth::logout))
        .route("/health", get(health::health))
        // Authenticated routes
        .route("/", get(|| async { Redirect::to("/accounts") }))
        .route("/accounts", get(accounts::list))
        .route("/accounts/add", get(accounts::add_form).post(accounts::add))
        // NIP-scoped routes (uses NipContext extractor)
        .route("/accounts/{nip}", get(dashboard::dashboard))
        .route(
            "/accounts/{nip}/invoices",
            get(invoices::list).post(invoices::create),
        )
        .route("/accounts/{nip}/invoices/new", get(invoices::new_form))
        .route(
            "/accounts/{nip}/invoices/fetch",
            get(fetch::fetch_form).post(fetch::fetch_execute),
        )
        .route("/accounts/{nip}/invoices/{id}", get(invoices::detail))
        .route(
            "/accounts/{nip}/invoices/{id}/submit",
            post(invoices::submit),
        )
        // Sessions
        .route(
            "/accounts/{nip}/sessions",
            get(sessions::sessions_page),
        )
        .route(
            "/accounts/{nip}/sessions/authenticate",
            post(sessions::authenticate),
        )
        .route(
            "/accounts/{nip}/sessions/close",
            post(sessions::close_session),
        )
        // Permissions
        .route(
            "/accounts/{nip}/permissions",
            get(permissions::permissions_page),
        )
        .route(
            "/accounts/{nip}/permissions/grant",
            post(permissions::grant),
        )
        .route(
            "/accounts/{nip}/permissions/revoke",
            post(permissions::revoke),
        )
        .route(
            "/accounts/{nip}/permissions/query",
            post(permissions::query),
        )
        // Tokens
        .route("/accounts/{nip}/tokens", get(tokens::tokens_page))
        .route(
            "/accounts/{nip}/tokens/generate",
            post(tokens::generate),
        )
        .route(
            "/accounts/{nip}/tokens/{token_id}/revoke",
            post(tokens::revoke),
        )
        // Export
        .route(
            "/accounts/{nip}/export",
            get(export::export_page).post(export::start_export),
        )
        .route(
            "/accounts/{nip}/export/{reference}",
            get(export::check_status),
        )
        .route(
            "/accounts/{nip}/export/{reference}/download",
            get(export::download),
        )
        // Strip trailing slashes: /foo/ → 308 redirect to /foo
        .fallback(trailing_slash_redirect)
}

async fn trailing_slash_redirect(req: Request) -> axum::response::Response {
    let path = req.uri().path();
    if path.len() > 1 && path.ends_with('/') {
        let new_path = path.trim_end_matches('/');
        let new_uri = if let Some(query) = req.uri().query() {
            format!("{new_path}?{query}")
        } else {
            new_path.to_string()
        };
        Redirect::permanent(&new_uri).into_response()
    } else {
        (axum::http::StatusCode::NOT_FOUND, "Nie znaleziono strony").into_response()
    }
}

use axum::response::IntoResponse;
