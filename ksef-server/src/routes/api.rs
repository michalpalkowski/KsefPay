use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use ksef_core::domain::nip::Nip;
use ksef_core::ports::company_lookup::CompanyLookupError;
use ksef_core::services::company_lookup_service::CompanyLookupServiceError;

use crate::state::AppState;

#[derive(Serialize)]
struct NipLookupResponse {
    found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bank_accounts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vat_status: Option<String>,
}

pub async fn nip_lookup(
    State(state): State<AppState>,
    Path(raw_nip): Path<String>,
) -> Response {
    let nip = match Nip::parse(&raw_nip) {
        Ok(nip) => nip,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(NipLookupResponse {
                    found: false,
                    name: None,
                    address: None,
                    bank_accounts: None,
                    vat_status: None,
                }),
            )
                .into_response();
        }
    };

    match state.company_lookup_service.lookup(&nip).await {
        Ok(info) => Json(NipLookupResponse {
            found: true,
            name: Some(info.name),
            address: Some(info.address),
            bank_accounts: Some(info.bank_accounts),
            vat_status: Some(info.vat_status.to_string()),
        })
        .into_response(),
        Err(e) => {
            let status = match &e {
                CompanyLookupServiceError::Lookup(CompanyLookupError::NotFound(_)) => {
                    StatusCode::OK
                }
                CompanyLookupServiceError::Lookup(CompanyLookupError::ApiError(_)) => {
                    StatusCode::BAD_GATEWAY
                }
                CompanyLookupServiceError::Cache(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(NipLookupResponse {
                    found: false,
                    name: None,
                    address: None,
                    bank_accounts: None,
                    vat_status: None,
                }),
            )
                .into_response()
        }
    }
}
