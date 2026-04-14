use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;

use crate::domain::environment::KSeFEnvironment;
use crate::error::{KSeFError, map_ksef_error_response};
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::{RetryPolicy, RetryableError};

/// Shared HTTP transport for all `KSeF` API clients.
///
/// Encapsulates `reqwest::Client`, base URL, rate limiting, and retry logic
/// so that individual client modules only define endpoint-specific behaviour.
pub struct KSeFHttpClient {
    pub(super) client: Client,
    pub(super) base_url: String,
    pub(super) rate_limiter: Arc<TokenBucketRateLimiter>,
    pub(super) retry_policy: RetryPolicy,
}

impl KSeFHttpClient {
    #[must_use]
    pub fn new(environment: KSeFEnvironment) -> Self {
        Self::with_http_controls(
            environment,
            Arc::new(TokenBucketRateLimiter::default()),
            RetryPolicy::default(),
        )
    }

    #[must_use]
    pub fn with_http_controls(
        environment: KSeFEnvironment,
        rate_limiter: Arc<TokenBucketRateLimiter>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url: environment.api_base_url().to_string(),
            rate_limiter,
            retry_policy,
        }
    }

    /// Send an HTTP request with rate limiting, retry, and error mapping.
    ///
    /// The `is_success` predicate decides which status codes count as success
    /// (most callers pass `StatusCode::is_success`, auth uses a custom check
    /// that also accepts 202).
    pub async fn send_with_retry<F, Fut, S>(
        &self,
        category: RateLimitCategory,
        mut request: F,
        is_success: S,
    ) -> Result<reqwest::Response, KSeFError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
        S: Fn(reqwest::StatusCode) -> bool + Copy,
    {
        let mut retries_done = 0u32;

        loop {
            self.rate_limiter.acquire(category).await?;

            let response = request().await?;
            if is_success(response.status()) {
                return Ok(response);
            }

            let error = map_http_error(response).await;
            if !error.is_retryable() || retries_done >= self.retry_policy.max_retries {
                return Err(error);
            }

            let delay_ms = self.retry_policy.next_delay_ms(retries_done, &error);
            retries_done += 1;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    /// Convenience wrapper — success = `StatusCode::is_success()`.
    pub async fn send<F, Fut>(
        &self,
        category: RateLimitCategory,
        request: F,
    ) -> Result<reqwest::Response, KSeFError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        self.send_with_retry(category, request, |s| s.is_success())
            .await
    }
}

// ---------------------------------------------------------------------------
// Shared JSON helpers — used by multiple client modules
// ---------------------------------------------------------------------------

pub async fn read_error_body(response: reqwest::Response) -> String {
    match response.text().await {
        Ok(body) => body,
        Err(err) => format!("<failed to read response body: {err}>"),
    }
}

pub async fn map_http_error(response: reqwest::Response) -> KSeFError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = read_error_body(response).await;
    map_ksef_error_response(status.as_u16(), retry_after.as_deref(), &body)
}

pub fn value_by_keys<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

pub fn str_by_keys<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    value_by_keys(value, keys).and_then(Value::as_str)
}

pub fn parse_dt(raw: &str, field: &str) -> Result<DateTime<Utc>, KSeFError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| KSeFError::StatusQueryFailed(format!("invalid {field} datetime: '{raw}'")))
}

pub fn u32_by_keys(value: &Value, keys: &[&str], field: &str) -> Result<u32, KSeFError> {
    let raw = value_by_keys(value, keys)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(format!("payload missing numeric field {field}"))
        })?;
    u32::try_from(raw).map_err(|_| {
        KSeFError::StatusQueryFailed(format!(
            "numeric field {field} does not fit into u32: {raw}"
        ))
    })
}

/// Parse public key PEM from a `KSeF` certificate JSON item.
pub fn parse_public_key_pem(item: &Value) -> Result<String, KSeFError> {
    if let Some(pem) = str_by_keys(item, &["keyPem", "publicKeyPem"]) {
        return Ok(pem.to_string());
    }
    let certificate_raw = str_by_keys(item, &["certificate"]).ok_or_else(|| {
        KSeFError::PublicKeyFetchFailed(
            "public key entry missing keyPem/publicKeyPem/certificate".to_string(),
        )
    })?;

    let cert_bytes = if certificate_raw.contains("-----BEGIN CERTIFICATE-----") {
        certificate_raw.as_bytes().to_vec()
    } else {
        openssl::base64::decode_block(certificate_raw).map_err(|e| {
            KSeFError::PublicKeyFetchFailed(format!("invalid certificate base64: {e}"))
        })?
    };
    let cert = openssl::x509::X509::from_pem(&cert_bytes)
        .or_else(|_| openssl::x509::X509::from_der(&cert_bytes))
        .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("invalid X509 certificate: {e}")))?;
    let public_key_pem = cert
        .public_key()
        .and_then(|key| key.public_key_to_pem())
        .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("extract public key: {e}")))?;
    String::from_utf8(public_key_pem)
        .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("public key pem utf8: {e}")))
}

/// Parse public key list from a `KSeF` response payload.
pub fn parse_public_keys(
    payload: &Value,
) -> Result<Vec<crate::domain::crypto::KSeFPublicKey>, KSeFError> {
    let items = if let Some(array) = payload.as_array() {
        array
    } else if let Some(items) =
        value_by_keys(payload, &["items", "publicKeyPemList", "result"]).and_then(Value::as_array)
    {
        items
    } else {
        return Err(KSeFError::PublicKeyFetchFailed(
            "unexpected public-key response format".to_string(),
        ));
    };

    let mut parsed = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if matches!(item.get("isActive").and_then(Value::as_bool), Some(false)) {
            continue;
        }
        if let Some(usage) = item.get("usage").and_then(Value::as_array) {
            let has_symmetric_usage = usage.iter().any(|entry| {
                entry
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case("SymmetricKeyEncryption"))
            });
            if !has_symmetric_usage {
                continue;
            }
        }

        let key_id = str_by_keys(item, &["keyId", "kid", "id"])
            .map_or_else(|| format!("ksef-public-key-{idx}"), str::to_string);
        let key_pem = parse_public_key_pem(item)?;
        parsed.push(crate::domain::crypto::KSeFPublicKey::new(key_pem, key_id));
    }

    if parsed.is_empty() {
        return Err(KSeFError::PublicKeyFetchFailed(
            "no usable public keys found in response".to_string(),
        ));
    }
    Ok(parsed)
}
