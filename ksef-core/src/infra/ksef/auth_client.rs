use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::domain::auth::{
    AccessToken, AuthChallenge, AuthReference, AuthStatus, ContextIdentifier, RefreshToken,
    TokenPair,
};
use crate::domain::crypto::SignedAuthRequest;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::nip::Nip;
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_auth::KSeFAuth;

use super::http_base::KSeFHttpClient;

/// HTTP implementation of `KSeFAuth` using `reqwest`.
pub struct HttpKSeFAuth {
    http: KSeFHttpClient,
}

impl HttpKSeFAuth {
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

// --- KSeF API response types ---

#[derive(Deserialize)]
struct ChallengeResponse {
    timestamp: String,
    challenge: String,
}

#[derive(Deserialize)]
struct AuthSubmitResponse {
    #[serde(alias = "referenceNumber")]
    reference_number: String,
    #[serde(alias = "authenticationToken")]
    authentication_token: Option<AuthOperationTokenResponse>,
    #[serde(alias = "token")]
    token: Option<String>,
}

#[derive(Deserialize)]
struct AuthOperationTokenResponse {
    token: String,
}

fn parse_auth_status_payload(payload: &Value) -> Result<AuthStatus, KSeFError> {
    let code = payload
        .get("processingCode")
        .and_then(Value::as_u64)
        .or_else(|| {
            payload
                .get("status")
                .and_then(Value::as_object)
                .and_then(|status| status.get("code"))
                .and_then(Value::as_u64)
        })
        .ok_or_else(|| {
            KSeFError::AuthPollingFailed(
                "auth status payload missing processing code/status.code".to_string(),
            )
        })?;

    let description = payload
        .get("processingDescription")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("status")
                .and_then(Value::as_object)
                .and_then(|status| status.get("description"))
                .and_then(Value::as_str)
        })
        .map_or_else(|| "<missing description>".to_string(), str::to_string);

    match code {
        200 => Ok(AuthStatus::Completed),
        100 => Ok(AuthStatus::Processing),
        other => Ok(AuthStatus::Failed {
            reason: format!("processing_code={other}: {description}"),
        }),
    }
}

fn parse_token_value(payload: &Value, key: &str) -> Option<String> {
    let field = payload.get(key)?;
    if let Some(token) = field.as_str() {
        return Some(token.to_string());
    }
    field
        .as_object()
        .and_then(|object| object.get("token"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn parse_valid_until_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_token_expiry(
    payload: &Value,
    token_key: &str,
    expires_in_key: &str,
) -> Option<DateTime<Utc>> {
    if let Some(seconds) = payload.get(expires_in_key).and_then(Value::as_i64) {
        return Some(Utc::now() + chrono::Duration::seconds(seconds));
    }

    payload
        .get(token_key)
        .and_then(Value::as_object)
        .and_then(|object| object.get("validUntil"))
        .and_then(Value::as_str)
        .and_then(parse_valid_until_utc)
}

fn parse_token_pair_payload(body: &str, error_context: &str) -> Result<TokenPair, KSeFError> {
    let payload: Value = serde_json::from_str(body).map_err(|e| {
        KSeFError::TokenRedeemFailed(format!("{error_context}: parse response: {e}; body={body}"))
    })?;

    let access_token = parse_token_value(&payload, "accessToken").ok_or_else(|| {
        KSeFError::TokenRedeemFailed(format!(
            "{error_context}: missing accessToken/token in response"
        ))
    })?;
    let refresh_token = parse_token_value(&payload, "refreshToken").ok_or_else(|| {
        KSeFError::TokenRedeemFailed(format!(
            "{error_context}: missing refreshToken/token in response"
        ))
    })?;

    let access_token_expires_at = parse_token_expiry(&payload, "accessToken", "expiresIn")
        .ok_or_else(|| {
            KSeFError::TokenRedeemFailed(format!(
                "{error_context}: missing access token expiry (expiresIn or accessToken.validUntil)"
            ))
        })?;
    let refresh_token_expires_at = parse_token_expiry(&payload, "refreshToken", "refreshExpiresIn")
        .ok_or_else(|| {
            KSeFError::TokenRedeemFailed(format!(
                "{error_context}: missing refresh token expiry (refreshExpiresIn or refreshToken.validUntil)"
            ))
        })?;

    Ok(TokenPair {
        access_token: AccessToken::new(access_token),
        refresh_token: RefreshToken::new(refresh_token),
        access_token_expires_at,
        refresh_token_expires_at,
    })
}

#[async_trait]
impl KSeFAuth for HttpKSeFAuth {
    async fn request_challenge(&self, nip: &Nip) -> Result<AuthChallenge, KSeFError> {
        let url = format!("{}/auth/challenge", self.http.base_url);

        let body = serde_json::json!({
            "contextIdentifier": {
                "type": "onip",
                "identifier": nip.as_str()
            }
        });

        let response = self
            .http
            .send_with_retry(
                RateLimitCategory::Auth,
                || self.http.client.post(&url).json(&body).send(),
                |status| status.is_success(),
            )
            .await?;

        let data: ChallengeResponse = response
            .json()
            .await
            .map_err(|e| KSeFError::ChallengeFailed(format!("parse response: {e}")))?;

        Ok(AuthChallenge {
            timestamp: data.timestamp,
            challenge: data.challenge,
        })
    }

    async fn authenticate_xades(
        &self,
        signed_request: &SignedAuthRequest,
    ) -> Result<AuthReference, KSeFError> {
        let url = format!("{}/auth/xades-signature", self.http.base_url);

        let response = self
            .http
            .send_with_retry(
                RateLimitCategory::Auth,
                || {
                    self.http
                        .client
                        .post(&url)
                        .header("Content-Type", "application/xml")
                        .body(signed_request.as_bytes().to_vec())
                        .send()
                },
                |status| status.is_success() || status.as_u16() == 202,
            )
            .await?;

        let data: AuthSubmitResponse = response
            .json()
            .await
            .map_err(|e| KSeFError::AuthPollingFailed(format!("parse response: {e}")))?;

        let auth_token = data
            .authentication_token
            .map(|token| token.token)
            .or(data.token)
            .ok_or_else(|| {
                KSeFError::AuthPollingFailed(
                    "missing authentication token in auth submit response".to_string(),
                )
            })?;

        Ok(AuthReference::new(data.reference_number, auth_token))
    }

    async fn authenticate_token(
        &self,
        context: &ContextIdentifier,
        token: &str,
    ) -> Result<AuthReference, KSeFError> {
        if token.trim().is_empty() {
            return Err(KSeFError::ChallengeFailed(
                "token authentication token cannot be empty".to_string(),
            ));
        }

        let url = format!("{}/auth/init-token-authentication", self.http.base_url);
        let payload = serde_json::json!({
            "contextIdentifier": {
                "type": context.api_type(),
                "identifier": context.value(),
            },
            "token": token,
            "timestamp": Utc::now().to_rfc3339(),
        });

        let response = self
            .http
            .send_with_retry(
                RateLimitCategory::Auth,
                || self.http.client.post(&url).json(&payload).send(),
                |status| status.is_success() || status.as_u16() == 202,
            )
            .await?;

        let data: AuthSubmitResponse = response
            .json()
            .await
            .map_err(|e| KSeFError::AuthPollingFailed(format!("parse response: {e}")))?;

        let auth_token = data
            .authentication_token
            .map(|token| token.token)
            .or(data.token)
            .ok_or_else(|| {
                KSeFError::AuthPollingFailed(
                    "missing authentication token in token-auth response".to_string(),
                )
            })?;

        Ok(AuthReference::new(data.reference_number, auth_token))
    }

    async fn poll_auth_status(&self, reference: &AuthReference) -> Result<AuthStatus, KSeFError> {
        let url = format!("{}/auth/{}", self.http.base_url, reference);

        let response = self
            .http
            .send_with_retry(
                RateLimitCategory::Auth,
                || {
                    self.http
                        .client
                        .get(&url)
                        .bearer_auth(reference.authentication_token())
                        .send()
                },
                |status| status.is_success(),
            )
            .await?;

        let body = response.text().await.map_err(|e| {
            KSeFError::AuthPollingFailed(format!("failed to read auth status body: {e}"))
        })?;
        let payload: Value = serde_json::from_str(&body).map_err(|e| {
            KSeFError::AuthPollingFailed(format!("parse response: {e}; body={body}"))
        })?;
        parse_auth_status_payload(&payload)
    }

    async fn redeem_token(&self, reference: &AuthReference) -> Result<TokenPair, KSeFError> {
        let url = format!("{}/auth/token/redeem", self.http.base_url);

        let response = self
            .http
            .send_with_retry(
                RateLimitCategory::Auth,
                || {
                    self.http
                        .client
                        .post(&url)
                        .bearer_auth(reference.authentication_token())
                        .send()
                },
                |status| status.is_success(),
            )
            .await?;

        let body = response.text().await.map_err(|e| {
            KSeFError::TokenRedeemFailed(format!("failed to read token redeem body: {e}"))
        })?;
        parse_token_pair_payload(&body, "token redeem")
    }

    async fn refresh_token(&self, refresh_token: &RefreshToken) -> Result<TokenPair, KSeFError> {
        let url = format!("{}/auth/token/refresh", self.http.base_url);

        let response = self
            .http
            .send_with_retry(
                RateLimitCategory::Auth,
                || {
                    self.http
                        .client
                        .post(&url)
                        .bearer_auth(refresh_token.as_str())
                        .send()
                },
                |status| status.is_success(),
            )
            .await?;

        let body = response.text().await.map_err(|e| {
            KSeFError::TokenRefreshFailed(format!("failed to read token refresh body: {e}"))
        })?;
        if let Ok(pair) = parse_token_pair_payload(&body, "token refresh") {
            Ok(pair)
        } else {
            let payload: Value = serde_json::from_str(&body).map_err(|e| {
                KSeFError::TokenRefreshFailed(format!("parse response: {e}; body={body}"))
            })?;
            let access_obj = payload
                .get("accessToken")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    KSeFError::TokenRefreshFailed(
                        "token refresh response missing accessToken object".to_string(),
                    )
                })?;
            let access_raw = access_obj
                .get("token")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    KSeFError::TokenRefreshFailed(
                        "token refresh response missing accessToken.token".to_string(),
                    )
                })?;
            let valid_until_raw = access_obj
                .get("validUntil")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    KSeFError::TokenRefreshFailed(
                        "token refresh response missing accessToken.validUntil".to_string(),
                    )
                })?;
            let access_token_expires_at = DateTime::parse_from_rfc3339(valid_until_raw)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|_| {
                    KSeFError::TokenRefreshFailed(format!(
                        "invalid accessToken.validUntil datetime: '{valid_until_raw}'"
                    ))
                })?;

            Ok(TokenPair {
                access_token: AccessToken::new(access_raw.to_string()),
                refresh_token: refresh_token.clone(),
                refresh_token_expires_at: Utc::now() + chrono::Duration::days(7),
                access_token_expires_at,
            })
        }
    }
}
