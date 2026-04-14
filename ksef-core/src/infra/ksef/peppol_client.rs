use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::peppol::PeppolProvider;
use crate::error::{DomainError, KSeFError};
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_peppol::{KSeFPeppol, PeppolProvidersResponse, PeppolQueryRequest};

use super::http_base::{KSeFHttpClient, str_by_keys, value_by_keys};

pub struct HttpKSeFPeppol {
    http: KSeFHttpClient,
}

impl HttpKSeFPeppol {
    #[must_use]
    pub fn new(environment: KSeFEnvironment) -> Self {
        Self {
            http: KSeFHttpClient::new(environment),
        }
    }

    #[must_use]
    pub fn with_http_controls(
        environment: KSeFEnvironment,
        rate_limiter: Arc<TokenBucketRateLimiter>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            http: KSeFHttpClient::with_http_controls(environment, rate_limiter, retry_policy),
        }
    }
}

fn parse_provider_item(item: &Value) -> Result<PeppolProvider, KSeFError> {
    let provider_id = str_by_keys(item, &["providerId", "id"]).ok_or_else(|| {
        KSeFError::StatusQueryFailed("provider missing providerId/id".to_string())
    })?;
    let name = str_by_keys(item, &["name", "providerName"]).ok_or_else(|| {
        KSeFError::StatusQueryFailed("provider missing name/providerName".to_string())
    })?;

    let provider = PeppolProvider {
        provider_id: provider_id.to_string(),
        name: name.to_string(),
        country_code: str_by_keys(item, &["countryCode", "country"])
            .unwrap_or("PL")
            .to_string(),
        endpoint_url: str_by_keys(item, &["endpointUrl", "url"])
            .unwrap_or("https://peppol.ksef.mf.gov.pl")
            .to_string(),
        active: value_by_keys(item, &["active", "isActive"])
            .and_then(Value::as_bool)
            .unwrap_or(true),
    };

    provider.validate().map_err(|err| match err {
        DomainError::InvalidParse { .. }
        | DomainError::InvalidNip { .. }
        | DomainError::InvalidStatusTransition { .. }
        | DomainError::InvalidAmount(_)
        | DomainError::InvalidVatRate(_) => KSeFError::StatusQueryFailed(format!(
            "invalid provider payload for '{}': {err}",
            provider.provider_id
        )),
    })?;

    Ok(provider)
}

fn parse_total(payload: &Value, item_count: usize) -> Result<u32, KSeFError> {
    if payload.is_array() {
        return u32::try_from(item_count).map_err(|_| {
            KSeFError::StatusQueryFailed("too many peppol providers in response".to_string())
        });
    }

    if let Some(total_u64) =
        value_by_keys(payload, &["total", "totalCount", "count"]).and_then(Value::as_u64)
    {
        return u32::try_from(total_u64).map_err(|_| {
            KSeFError::StatusQueryFailed(format!("peppol total is too large for u32: {total_u64}"))
        });
    }

    u32::try_from(item_count).map_err(|_| {
        KSeFError::StatusQueryFailed("too many peppol providers in response".to_string())
    })
}

fn parse_query_response(payload: &Value) -> Result<PeppolProvidersResponse, KSeFError> {
    let items = if let Some(array) = payload.as_array() {
        array
    } else if let Some(array) = value_by_keys(
        payload,
        &["peppolProviders", "items", "providers", "result"],
    )
    .and_then(Value::as_array)
    {
        array
    } else {
        return Err(KSeFError::StatusQueryFailed(
            "unexpected peppol query response format".to_string(),
        ));
    };

    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        parsed.push(parse_provider_item(item)?);
    }

    let total = parse_total(payload, parsed.len())?;
    Ok(PeppolProvidersResponse {
        items: parsed,
        total,
    })
}

#[async_trait]
impl KSeFPeppol for HttpKSeFPeppol {
    async fn query_providers(
        &self,
        access_token: &AccessToken,
        request: &PeppolQueryRequest,
    ) -> Result<PeppolProvidersResponse, KSeFError> {
        let url = format!(
            "{}/peppol/query?pageOffset={}&pageSize={}",
            self.http.base_url, request.page_offset, request.page_size
        );
        let response = self
            .http
            .send(RateLimitCategory::Query, || {
                self.http
                    .client
                    .get(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;

        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse peppol query response: {e}"))
        })?;

        parse_query_response(&payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_response_accepts_valid_payload() {
        let payload = serde_json::json!({
            "peppolProviders": [{
                "id": "p1",
                "name": "Provider",
                "dateCreated": "2026-04-13T10:00:00Z"
            }],
            "hasMore": false
        });

        let parsed = parse_query_response(&payload).unwrap();
        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].provider_id, "p1");
    }

    #[test]
    fn parse_query_response_uses_item_count_when_total_missing() {
        let payload = serde_json::json!({
            "peppolProviders": [{
                "id": "p1",
                "name": "Provider",
                "dateCreated": "2026-04-13T10:00:00Z"
            }]
        });

        let parsed = parse_query_response(&payload).unwrap();
        assert_eq!(parsed.total, 1);
    }
}
