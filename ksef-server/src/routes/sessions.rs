use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use ksef_core::domain::nip::Nip;

use crate::extractors::NipContext;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/sessions.html")]
struct SessionsTemplate {
    active: &'static str,
    nip_prefix: Option<String>,
    user_email: String,
    has_token: bool,
    has_session: bool,
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

async fn session_state(state: &AppState, nip: &Nip) -> (bool, bool) {
    let has_token = state.session_service.has_valid_token(nip).await;
    let has_session = state.session_service.has_active_session(nip).await;
    (has_token, has_session)
}

pub async fn sessions_page(State(state): State<AppState>, nip_ctx: NipContext) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    let (has_token, has_session) = session_state(&state, nip).await;
    render(SessionsTemplate {
        active: "/sessions",
        nip_prefix: Some(nip_str),
        user_email: nip_ctx.user.email,
        has_token,
        has_session,
        error: None,
        success: None,
    })
}

pub async fn authenticate(State(state): State<AppState>, nip_ctx: NipContext) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    match state.session_service.authenticate(nip).await {
        Ok(_token_pair) => {
            let (has_token, has_session) = session_state(&state, nip).await;
            render(SessionsTemplate {
                active: "/sessions",
                nip_prefix: Some(nip_str),
                user_email: nip_ctx.user.email,
                has_token,
                has_session,
                error: None,
                success: Some("Uwierzytelnienie zakonczone pomyslnie".to_string()),
            })
        }
        Err(e) => {
            let (has_token, has_session) = session_state(&state, nip).await;
            render(SessionsTemplate {
                active: "/sessions",
                nip_prefix: Some(nip_str),
                user_email: nip_ctx.user.email,
                has_token,
                has_session,
                error: Some(format!("Uwierzytelnienie nie powiodło się: {e}")),
                success: None,
            })
        }
    }
}

pub async fn close_session(State(state): State<AppState>, nip_ctx: NipContext) -> Response {
    let nip = &nip_ctx.account.nip;
    let nip_str = nip.to_string();
    match state.session_service.close_session(nip).await {
        Ok(_upo) => {
            let (has_token, has_session) = session_state(&state, nip).await;
            render(SessionsTemplate {
                active: "/sessions",
                nip_prefix: Some(nip_str),
                user_email: nip_ctx.user.email,
                has_token,
                has_session,
                error: None,
                success: Some("Sesja zamknięta".to_string()),
            })
        }
        Err(e) => {
            let (has_token, has_session) = session_state(&state, nip).await;
            render(SessionsTemplate {
                active: "/sessions",
                nip_prefix: Some(nip_str),
                user_email: nip_ctx.user.email,
                has_token,
                has_session,
                error: Some(format!("Zamknięcie sesji nie powiodło się: {e}")),
                success: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_template(has_token: bool, has_session: bool, error: Option<String>, success: Option<String>) -> SessionsTemplate {
        SessionsTemplate {
            active: "/sessions",
            nip_prefix: Some("5260250274".to_string()),
            user_email: "test@example.com".to_string(),
            has_token,
            has_session,
            error,
            success,
        }
    }

    #[test]
    fn template_unauthenticated_hides_close_button() {
        let tmpl = test_template(false, false, None, None);
        let html = tmpl.render().unwrap();
        assert!(!html.contains("Zamknij sesje"));
        assert!(html.contains("Uwierzytelnij"));
    }

    #[test]
    fn template_with_session_shows_close_button() {
        let tmpl = test_template(true, true, None, None);
        let html = tmpl.render().unwrap();
        assert!(html.contains("Zamknij sesje"));
    }

    #[test]
    fn template_shows_error_alert() {
        let tmpl = test_template(false, false, Some("Auth failed".to_string()), None);
        let html = tmpl.render().unwrap();
        assert!(html.contains("Auth failed"));
        assert!(html.contains("alert-error"));
    }

    #[test]
    fn template_token_only_shows_correct_message() {
        let tmpl = test_template(true, false, None, None);
        let html = tmpl.render().unwrap();
        assert!(html.contains("Sesja online otwiera sie automatycznie"));
        assert!(html.contains("Odnow token"));
    }
}
