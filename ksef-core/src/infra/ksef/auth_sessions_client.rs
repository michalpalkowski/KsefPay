use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::auth::{AccessToken, AuthSessionInfo};
use crate::domain::environment::KSeFEnvironment;
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_auth_sessions::KSeFAuthSessions;

use super::http_base::{KSeFHttpClient, parse_dt, str_by_keys, value_by_keys};

pub struct HttpKSeFAuthSessions {
    http: KSeFHttpClient,
}

impl HttpKSeFAuthSessions {
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

fn parse_auth_sessions(payload: &Value) -> Result<Vec<AuthSessionInfo>, KSeFError> {
    let items = if let Some(array) = payload.as_array() {
        array
    } else if let Some(array) =
        value_by_keys(payload, &["items", "sessions", "result"]).and_then(Value::as_array)
    {
        array
    } else {
        return Err(KSeFError::StatusQueryFailed(
            "unexpected auth-sessions response format".to_string(),
        ));
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let reference_number = str_by_keys(item, &["referenceNumber", "reference", "id"])
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "auth-session item missing referenceNumber/reference/id".to_string(),
                )
            })?;
        let created_raw = str_by_keys(item, &["startDate", "createdAt", "created", "timestamp"])
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "auth-session item missing startDate/createdAt/created/timestamp".to_string(),
                )
            })?;
        let current = value_by_keys(item, &["isCurrent", "current"])
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.push(AuthSessionInfo {
            reference_number: reference_number.to_string(),
            created_at: parse_dt(created_raw, "createdAt")?,
            current,
        });
    }
    Ok(out)
}

#[async_trait]
impl KSeFAuthSessions for HttpKSeFAuthSessions {
    async fn list_sessions(
        &self,
        access_token: &AccessToken,
    ) -> Result<Vec<AuthSessionInfo>, KSeFError> {
        let url = format!("{}/auth/sessions", self.http.base_url);
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
            KSeFError::StatusQueryFailed(format!("parse auth-sessions response: {e}"))
        })?;
        parse_auth_sessions(&payload)
    }

    async fn revoke_session(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<(), KSeFError> {
        if reference_number.trim().is_empty() {
            return Err(KSeFError::StatusQueryFailed(
                "auth-session reference number cannot be empty".to_string(),
            ));
        }

        let url = format!("{}/auth/sessions/{reference_number}", self.http.base_url);
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

    async fn revoke_current_session(&self, access_token: &AccessToken) -> Result<(), KSeFError> {
        let url = format!("{}/auth/sessions/current", self.http.base_url);
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
    fn parse_auth_sessions_accepts_valid_items() {
        let payload = serde_json::json!({
            "items": [{
                "referenceNumber": "s1",
                "createdAt": "2026-04-13T10:00:00Z",
                "current": true
            }]
        });
        let parsed = parse_auth_sessions(&payload).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].current);
    }

    #[test]
    fn parse_auth_sessions_rejects_missing_reference() {
        let payload = serde_json::json!({
            "items": [{
                "createdAt": "2026-04-13T10:00:00Z",
                "current": false
            }]
        });
        let err = parse_auth_sessions(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }
}
