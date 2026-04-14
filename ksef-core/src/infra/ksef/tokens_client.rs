use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::permission::PermissionType;
use crate::domain::token_mgmt::{ManagedToken, TokenStatus};
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_tokens::{
    KSeFTokens, TokenGenerateRequest, TokenQueryRequest, TokenQueryResponse,
};

use super::http_base::{KSeFHttpClient, parse_dt, str_by_keys, value_by_keys};

pub struct HttpKSeFTokens {
    http: KSeFHttpClient,
}

impl HttpKSeFTokens {
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

fn parse_permission(raw: &str) -> Result<PermissionType, KSeFError> {
    raw.parse::<PermissionType>().map_err(|_| {
        KSeFError::StatusQueryFailed(format!("invalid permission value in token: '{raw}'"))
    })
}

fn parse_permissions(value: &Value) -> Result<Vec<PermissionType>, KSeFError> {
    let permissions_value = value_by_keys(
        value,
        &["permissions", "requestedPermissions", "permissionList"],
    )
    .ok_or_else(|| {
        KSeFError::StatusQueryFailed(
            "token item missing permissions/requestedPermissions/permissionList".to_string(),
        )
    })?;
    let permission_items = permissions_value.as_array().ok_or_else(|| {
        KSeFError::StatusQueryFailed("token permissions should be an array".to_string())
    })?;

    let mut permissions = Vec::with_capacity(permission_items.len());
    for item in permission_items {
        let permission_raw = item
            .as_str()
            .or_else(|| str_by_keys(item, &["permissionType", "permission"]))
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "permission entry missing permissionType/permission".to_string(),
                )
            })?;
        permissions.push(parse_permission(permission_raw)?);
    }

    Ok(permissions)
}

fn parse_status_value(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .as_object()
            .and_then(|obj| obj.get("value"))
            .and_then(Value::as_str)
    })
}

fn parse_status(raw: &str) -> Result<TokenStatus, KSeFError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" | "active" => Ok(TokenStatus::Active),
        "revoking" | "revoked" => Ok(TokenStatus::Revoked),
        "failed" | "expired" => Ok(TokenStatus::Expired),
        _ => Err(KSeFError::StatusQueryFailed(format!(
            "invalid token status: '{raw}'"
        ))),
    }
}

fn parse_token_item(value: &Value) -> Result<ManagedToken, KSeFError> {
    let id = str_by_keys(value, &["referenceNumber", "tokenId", "id"]).ok_or_else(|| {
        KSeFError::StatusQueryFailed("token item missing tokenId/id/referenceNumber".to_string())
    })?;
    let status_raw = value_by_keys(value, &["status", "tokenStatus"])
        .and_then(parse_status_value)
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed("token item missing status/tokenStatus".to_string())
        })?;
    let created_at_raw = str_by_keys(
        value,
        &["dateCreated", "createdAt", "creationDate", "timestamp"],
    )
    .ok_or_else(|| {
        KSeFError::StatusQueryFailed(
            "token item missing dateCreated/createdAt/creationDate/timestamp".to_string(),
        )
    })?;
    let revoked_at = str_by_keys(value, &["revokedAt"])
        .map(|raw| parse_dt(raw, "revokedAt"))
        .transpose()?;
    let created_at = parse_dt(created_at_raw, "createdAt")?;
    let expires_at = str_by_keys(value, &["expiresAt", "validTo", "expirationDate"])
        .map(|raw| parse_dt(raw, "expiresAt"))
        .transpose()?
        .unwrap_or(created_at + chrono::Duration::days(365));

    Ok(ManagedToken {
        id: id.to_string(),
        status: parse_status(status_raw)?,
        permissions: parse_permissions(value)?,
        created_at,
        expires_at,
        revoked_at,
    })
}

fn parse_token_from_payload(payload: &Value) -> Result<ManagedToken, KSeFError> {
    if payload.is_object()
        && let Some(item) = value_by_keys(payload, &["token", "item", "result"])
    {
        return parse_token_item(item);
    }
    parse_token_item(payload)
}

fn parse_total(payload: &Value, item_count: usize) -> Result<u32, KSeFError> {
    if payload.is_array() {
        return u32::try_from(item_count).map_err(|_| {
            KSeFError::StatusQueryFailed("too many token records in response".to_string())
        });
    }

    if let Some(total_u64) =
        value_by_keys(payload, &["total", "totalCount", "count"]).and_then(Value::as_u64)
    {
        return u32::try_from(total_u64).map_err(|_| {
            KSeFError::StatusQueryFailed(format!(
                "query tokens total is too large for u32: {total_u64}"
            ))
        });
    }

    u32::try_from(item_count)
        .map_err(|_| KSeFError::StatusQueryFailed("too many token records in response".to_string()))
}

fn parse_token_query_response(payload: &Value) -> Result<TokenQueryResponse, KSeFError> {
    let items = if let Some(array) = payload.as_array() {
        array
    } else if let Some(array) =
        value_by_keys(payload, &["tokens", "items", "result"]).and_then(Value::as_array)
    {
        array
    } else {
        return Err(KSeFError::StatusQueryFailed(
            "unexpected query tokens response format".to_string(),
        ));
    };

    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        parsed.push(parse_token_item(item)?);
    }

    let total = parse_total(payload, parsed.len())?;
    Ok(TokenQueryResponse {
        items: parsed,
        total,
    })
}

#[async_trait]
impl KSeFTokens for HttpKSeFTokens {
    async fn generate_token(
        &self,
        access_token: &AccessToken,
        request: &TokenGenerateRequest,
    ) -> Result<ManagedToken, KSeFError> {
        let url = format!("{}/tokens", self.http.base_url);
        let permissions: Vec<String> = request
            .permissions
            .iter()
            .map(ToString::to_string)
            .collect();
        let description = request
            .description
            .as_deref()
            .filter(|s| s.len() >= 5)
            .unwrap_or("Token wygenerowany przez ksef-paymoney");
        let body = serde_json::json!({
            "permissions": permissions,
            "description": description,
        });

        let response = self
            .http
            .send(RateLimitCategory::Session, || {
                self.http
                    .client
                    .post(&url)
                    .bearer_auth(access_token.as_str())
                    .json(&body)
                    .send()
            })
            .await?;

        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse generate token response: {e}"))
        })?;
        let reference_number =
            str_by_keys(&payload, &["referenceNumber", "id"]).ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "generate token response missing referenceNumber".to_string(),
                )
            })?;

        self.get_token(access_token, reference_number).await
    }

    async fn query_tokens(
        &self,
        access_token: &AccessToken,
        request: &TokenQueryRequest,
    ) -> Result<TokenQueryResponse, KSeFError> {
        let mut url = reqwest::Url::parse(&format!("{}/tokens", self.http.base_url))
            .map_err(|e| KSeFError::StatusQueryFailed(format!("invalid tokens URL: {e}")))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(status) = request.status {
                query.append_pair("status", &status.to_string());
            }
            if let Some(limit) = request.limit {
                query.append_pair("pageSize", &limit.to_string());
            }
        }

        let response = self
            .http
            .send(RateLimitCategory::Query, || {
                self.http
                    .client
                    .get(url.clone())
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;

        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse query tokens response: {e}"))
        })?;

        parse_token_query_response(&payload)
    }

    async fn get_token(
        &self,
        access_token: &AccessToken,
        token_id: &str,
    ) -> Result<ManagedToken, KSeFError> {
        let url = format!("{}/tokens/{token_id}", self.http.base_url);
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

        let payload: Value = response
            .json()
            .await
            .map_err(|e| KSeFError::StatusQueryFailed(format!("parse get token response: {e}")))?;

        parse_token_from_payload(&payload)
    }

    async fn revoke_token(
        &self,
        access_token: &AccessToken,
        token_id: &str,
    ) -> Result<(), KSeFError> {
        let url = format!("{}/tokens/{token_id}", self.http.base_url);
        self.http
            .send(RateLimitCategory::Session, || {
                self.http
                    .client
                    .request(reqwest::Method::DELETE, &url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_item_accepts_valid_payload() {
        let payload = serde_json::json!({
            "tokenId": "token-1",
            "status": "active",
            "permissions": ["InvoiceRead", "CredentialsManage"],
            "createdAt": "2026-04-13T10:00:00Z",
            "expiresAt": "2026-04-20T10:00:00Z",
            "revokedAt": null,
        });

        let parsed = parse_token_item(&payload).unwrap();
        assert_eq!(parsed.id, "token-1");
        assert_eq!(parsed.status, TokenStatus::Active);
        assert_eq!(parsed.permissions.len(), 2);
        assert!(parsed.revoked_at.is_none());
    }

    #[test]
    fn parse_token_item_rejects_missing_fields() {
        let payload = serde_json::json!({
            "status": "active",
            "permissions": ["InvoiceRead"],
            "createdAt": "2026-04-13T10:00:00Z",
            "expiresAt": "2026-04-20T10:00:00Z",
        });

        let err = parse_token_item(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }

    #[test]
    fn parse_token_query_response_falls_back_to_item_count_when_total_missing() {
        let payload = serde_json::json!({
            "tokens": [{
                "referenceNumber": "token-1",
                "status": "Active",
                "requestedPermissions": ["InvoiceRead"],
                "dateCreated": "2026-04-13T10:00:00Z"
            }]
        });

        let parsed = parse_token_query_response(&payload).unwrap();
        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.items.len(), 1);
    }
}
