use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::rate_limit::{ContextLimits, EffectiveApiRateLimits, SubjectLimits};
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_rate_limits::KSeFRateLimits;

use super::http_base::{KSeFHttpClient, u32_by_keys, value_by_keys};

pub struct HttpKSeFRateLimits {
    http: KSeFHttpClient,
}

impl HttpKSeFRateLimits {
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

fn parse_effective_limits(payload: &Value) -> Result<EffectiveApiRateLimits, KSeFError> {
    let source = if payload.is_object() {
        value_by_keys(payload, &["rateLimits", "limits", "result"]).unwrap_or(payload)
    } else {
        payload
    };

    let category_map: [(&str, crate::domain::rate_limit::RateLimitCategory); 12] = [
        (
            "onlineSession",
            crate::domain::rate_limit::RateLimitCategory::Session,
        ),
        (
            "batchSession",
            crate::domain::rate_limit::RateLimitCategory::Session,
        ),
        (
            "invoiceSend",
            crate::domain::rate_limit::RateLimitCategory::Invoice,
        ),
        (
            "invoiceStatus",
            crate::domain::rate_limit::RateLimitCategory::Invoice,
        ),
        (
            "sessionList",
            crate::domain::rate_limit::RateLimitCategory::Session,
        ),
        (
            "sessionInvoiceList",
            crate::domain::rate_limit::RateLimitCategory::Query,
        ),
        (
            "sessionMisc",
            crate::domain::rate_limit::RateLimitCategory::Session,
        ),
        (
            "invoiceMetadata",
            crate::domain::rate_limit::RateLimitCategory::Query,
        ),
        (
            "invoiceExport",
            crate::domain::rate_limit::RateLimitCategory::Query,
        ),
        (
            "invoiceExportStatus",
            crate::domain::rate_limit::RateLimitCategory::Query,
        ),
        (
            "invoiceDownload",
            crate::domain::rate_limit::RateLimitCategory::Query,
        ),
        (
            "other",
            crate::domain::rate_limit::RateLimitCategory::Default,
        ),
    ];

    let mut contexts = Vec::with_capacity(category_map.len());
    for (field, category) in category_map {
        if let Some(item) = source.get(field) {
            let per_second = u32_by_keys(item, &["perSecond"], &format!("{field}.perSecond"))?;
            let per_minute = u32_by_keys(item, &["perMinute"], &format!("{field}.perMinute"))?;
            let per_hour = u32_by_keys(item, &["perHour"], &format!("{field}.perHour"))?;
            contexts.push(ContextLimits {
                category,
                per_second,
                per_minute,
                per_hour,
                burst: per_second,
            });
        }
    }

    if contexts.is_empty() {
        return Err(KSeFError::StatusQueryFailed(
            "effective rate-limits response missing known categories".to_string(),
        ));
    }

    Ok(EffectiveApiRateLimits {
        contexts,
        subjects: Vec::<SubjectLimits>::new(),
    })
}

#[async_trait]
impl KSeFRateLimits for HttpKSeFRateLimits {
    async fn get_effective_limits(
        &self,
        access_token: &AccessToken,
    ) -> Result<EffectiveApiRateLimits, KSeFError> {
        let url = format!("{}/rate-limits", self.http.base_url);
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
            KSeFError::StatusQueryFailed(format!("parse effective limits response: {e}"))
        })?;

        parse_effective_limits(&payload)
    }

    async fn get_context_limits(
        &self,
        access_token: &AccessToken,
    ) -> Result<Vec<ContextLimits>, KSeFError> {
        Ok(self.get_effective_limits(access_token).await?.contexts)
    }

    async fn get_subject_limits(
        &self,
        access_token: &AccessToken,
    ) -> Result<Vec<SubjectLimits>, KSeFError> {
        Ok(self.get_effective_limits(access_token).await?.subjects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_effective_limits_requires_contexts_and_subjects() {
        let payload = serde_json::json!({});
        let err = parse_effective_limits(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }
}
