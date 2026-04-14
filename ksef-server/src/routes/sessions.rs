use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/sessions.html")]
struct SessionsTemplate {
    active: &'static str,
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

async fn session_state(state: &AppState) -> (bool, bool) {
    let has_token = state.session_service.has_valid_token(&state.nip).await;
    let has_session = state.session_service.has_active_session(&state.nip).await;
    (has_token, has_session)
}

pub async fn sessions_page(State(state): State<AppState>) -> Response {
    let (has_token, has_session) = session_state(&state).await;
    render(SessionsTemplate {
        active: "/sessions",
        has_token,
        has_session,
        error: None,
        success: None,
    })
}

pub async fn authenticate(State(state): State<AppState>) -> Response {
    match state.session_service.authenticate(&state.nip).await {
        Ok(_token_pair) => {
            let (has_token, has_session) = session_state(&state).await;
            render(SessionsTemplate {
                active: "/sessions",
                has_token,
                has_session,
                error: None,
                success: Some("Uwierzytelnienie zakonczone pomyslnie".to_string()),
            })
        }
        Err(e) => {
            let (has_token, has_session) = session_state(&state).await;
            render(SessionsTemplate {
                active: "/sessions",
                has_token,
                has_session,
                error: Some(format!("Uwierzytelnienie nie powiodlo sie: {e}")),
                success: None,
            })
        }
    }
}

pub async fn close_session(State(state): State<AppState>) -> Response {
    match state.session_service.close_session(&state.nip).await {
        Ok(_upo) => {
            let (has_token, has_session) = session_state(&state).await;
            render(SessionsTemplate {
                active: "/sessions",
                has_token,
                has_session,
                error: None,
                success: Some("Sesja zamknieta".to_string()),
            })
        }
        Err(e) => {
            let (has_token, has_session) = session_state(&state).await;
            render(SessionsTemplate {
                active: "/sessions",
                has_token,
                has_session,
                error: Some(format!("Zamkniecie sesji nie powiodlo sie: {e}")),
                success: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_unauthenticated_hides_close_button() {
        let tmpl = SessionsTemplate {
            active: "/sessions",
            has_token: false,
            has_session: false,
            error: None,
            success: None,
        };
        let html = tmpl.render().unwrap();
        assert!(!html.contains("Zamknij sesje"));
        assert!(html.contains("Uwierzytelnij"));
    }

    #[test]
    fn template_with_session_shows_close_button() {
        let tmpl = SessionsTemplate {
            active: "/sessions",
            has_token: true,
            has_session: true,
            error: None,
            success: None,
        };
        let html = tmpl.render().unwrap();
        assert!(html.contains("Zamknij sesje"));
    }

    #[test]
    fn template_shows_error_alert() {
        let tmpl = SessionsTemplate {
            active: "/sessions",
            has_token: false,
            has_session: false,
            error: Some("Auth failed".to_string()),
            success: None,
        };
        let html = tmpl.render().unwrap();
        assert!(html.contains("Auth failed"));
        assert!(html.contains("alert-error"));
    }

    #[test]
    fn template_token_only_shows_correct_message() {
        let tmpl = SessionsTemplate {
            active: "/sessions",
            has_token: true,
            has_session: false,
            error: None,
            success: None,
        };
        let html = tmpl.render().unwrap();
        assert!(html.contains("Sesja online otwiera sie automatycznie"));
        assert!(html.contains("Odnow token"));
    }
}
