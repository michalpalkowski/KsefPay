use axum::Router;
use axum::routing::{get, post};

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
        .route("/", get(dashboard::dashboard))
        .route("/health", get(health::health))
        // Invoices
        .route("/invoices", get(invoices::list).post(invoices::create))
        .route("/invoices/new", get(invoices::new_form))
        .route(
            "/invoices/fetch",
            get(fetch::fetch_form).post(fetch::fetch_execute),
        )
        .route("/invoices/{id}", get(invoices::detail))
        .route("/invoices/{id}/submit", post(invoices::submit))
        // Sessions
        .route("/sessions", get(sessions::sessions_page))
        .route("/sessions/authenticate", post(sessions::authenticate))
        .route("/sessions/close", post(sessions::close_session))
        // Permissions
        .route("/permissions", get(permissions::permissions_page))
        .route("/permissions/grant", post(permissions::grant))
        .route("/permissions/revoke", post(permissions::revoke))
        .route("/permissions/query", post(permissions::query))
        // Tokens
        .route("/tokens", get(tokens::tokens_page))
        .route("/tokens/generate", post(tokens::generate))
        .route("/tokens/{token_id}/revoke", post(tokens::revoke))
        // Export
        .route(
            "/export",
            get(export::export_page).post(export::start_export),
        )
        .route("/export/{reference}", get(export::check_status))
        .route("/export/{reference}/download", get(export::download))
}
