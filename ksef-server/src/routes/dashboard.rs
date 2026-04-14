use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use ksef_core::ports::invoice_repository::InvoiceFilter;

use crate::state::AppState;

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
struct DashboardTemplate {
    active: &'static str,
    total_invoices: usize,
    draft_count: usize,
    queued_count: usize,
    accepted_count: usize,
    failed_count: usize,
}

pub async fn dashboard(State(state): State<AppState>) -> Response {
    let all = match state.invoice_service.list(&InvoiceFilter::default()).await {
        Ok(invoices) => invoices,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Nie udalo sie pobrac statystyk dashboardu: {err}"),
            )
                .into_response();
        }
    };

    let draft_count = all
        .iter()
        .filter(|i| i.status == ksef_core::domain::invoice::InvoiceStatus::Draft)
        .count();
    let queued_count = all
        .iter()
        .filter(|i| {
            matches!(
                i.status,
                ksef_core::domain::invoice::InvoiceStatus::Queued
                    | ksef_core::domain::invoice::InvoiceStatus::Submitted
            )
        })
        .count();
    let accepted_count = all
        .iter()
        .filter(|i| i.status == ksef_core::domain::invoice::InvoiceStatus::Accepted)
        .count();
    let failed_count = all
        .iter()
        .filter(|i| {
            matches!(
                i.status,
                ksef_core::domain::invoice::InvoiceStatus::Failed
                    | ksef_core::domain::invoice::InvoiceStatus::Rejected
            )
        })
        .count();

    let tmpl = DashboardTemplate {
        active: "/",
        total_invoices: all.len(),
        draft_count,
        queued_count,
        accepted_count,
        failed_count,
    };

    match tmpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {e}"),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_renders_with_zero_counts() {
        let tmpl = DashboardTemplate {
            active: "/",
            total_invoices: 0,
            draft_count: 0,
            queued_count: 0,
            accepted_count: 0,
            failed_count: 0,
        };
        let html = tmpl.render().unwrap();
        assert!(html.contains("Dashboard"));
    }

    #[test]
    fn template_renders_counts_correctly() {
        let tmpl = DashboardTemplate {
            active: "/",
            total_invoices: 42,
            draft_count: 5,
            queued_count: 3,
            accepted_count: 30,
            failed_count: 4,
        };
        let html = tmpl.render().unwrap();
        assert!(html.contains(">42<"));
        assert!(html.contains(">30<"));
    }
}
