use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::certificate_mgmt::{
    CertificateEnrollment, CertificateLimits, EnrollmentStatus, KsefCertificateType,
};
use crate::domain::environment::KSeFEnvironment;
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_certificates::{
    CertificateEnrollmentRequest, CertificateQueryRequest, KSeFCertificates,
};

use super::http_base::{KSeFHttpClient, parse_dt, str_by_keys, u32_by_keys, value_by_keys};

pub struct HttpKSeFCertificates {
    http: KSeFHttpClient,
}

impl HttpKSeFCertificates {
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

fn parse_certificate_type(raw: &str) -> Result<KsefCertificateType, KSeFError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "seal" => Ok(KsefCertificateType::Seal),
        "token" | "authentication" => Ok(KsefCertificateType::Token),
        "offline" => Ok(KsefCertificateType::Offline),
        other => Err(KSeFError::StatusQueryFailed(format!(
            "invalid certificate type in response: '{other}'"
        ))),
    }
}

fn parse_enrollment_status(raw: &str) -> Result<EnrollmentStatus, KSeFError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Ok(EnrollmentStatus::Pending),
        "approved" | "active" => Ok(EnrollmentStatus::Approved),
        "rejected" => Ok(EnrollmentStatus::Rejected),
        "revoked" | "inactive" => Ok(EnrollmentStatus::Revoked),
        _ => Err(KSeFError::StatusQueryFailed(format!(
            "invalid enrollment status in response: '{raw}'"
        ))),
    }
}

fn parse_enrollment_item(item: &Value) -> Result<CertificateEnrollment, KSeFError> {
    let reference_number = str_by_keys(
        item,
        &[
            "referenceNumber",
            "certificateSerialNumber",
            "reference",
            "id",
        ],
    )
    .ok_or_else(|| {
        KSeFError::StatusQueryFailed(
            "certificate item missing referenceNumber/certificateSerialNumber/reference/id"
                .to_string(),
        )
    })?;
    let certificate_type_raw =
        str_by_keys(item, &["certificateType", "type"]).ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "certificate item missing certificateType/type".to_string(),
            )
        })?;
    let status_raw = str_by_keys(item, &["status", "enrollmentStatus"]).ok_or_else(|| {
        KSeFError::StatusQueryFailed("certificate item missing status/enrollmentStatus".to_string())
    })?;
    let submitted_at_raw = str_by_keys(item, &["submittedAt", "createdAt", "requestDate"])
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "certificate item missing submittedAt/createdAt/requestDate".to_string(),
            )
        })?;
    let updated_at_raw = str_by_keys(
        item,
        &["updatedAt", "modifiedAt", "lastUseDate", "validFrom", "validTo", "requestDate"],
    )
    .ok_or_else(|| {
        KSeFError::StatusQueryFailed(
            "certificate item missing updatedAt/modifiedAt/lastUseDate/validFrom/validTo/requestDate"
                .to_string(),
        )
    })?;

    Ok(CertificateEnrollment {
        reference_number: reference_number.to_string(),
        certificate_type: parse_certificate_type(certificate_type_raw)?,
        status: parse_enrollment_status(status_raw)?,
        submitted_at: parse_dt(submitted_at_raw, "submittedAt")?,
        updated_at: parse_dt(updated_at_raw, "updatedAt")?,
    })
}

fn parse_enrollment_from_payload(payload: &Value) -> Result<CertificateEnrollment, KSeFError> {
    if payload.is_object()
        && let Some(item) = value_by_keys(payload, &["enrollment", "item", "result"])
    {
        return parse_enrollment_item(item);
    }
    parse_enrollment_item(payload)
}

fn parse_enrollment_list(payload: &Value) -> Result<Vec<CertificateEnrollment>, KSeFError> {
    let items = if let Some(array) = payload.as_array() {
        array
    } else if let Some(array) =
        value_by_keys(payload, &["certificates", "items", "result"]).and_then(Value::as_array)
    {
        array
    } else {
        return Err(KSeFError::StatusQueryFailed(
            "unexpected certificates query response format".to_string(),
        ));
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(parse_enrollment_item(item)?);
    }
    Ok(out)
}

fn parse_limits(payload: &Value) -> Result<CertificateLimits, KSeFError> {
    let source = if payload.is_object() {
        value_by_keys(payload, &["limits", "result"]).unwrap_or(payload)
    } else {
        payload
    };

    let certificate = value_by_keys(source, &["certificate"]).ok_or_else(|| {
        KSeFError::StatusQueryFailed("certificate payload missing certificate section".to_string())
    })?;
    let enrollment = value_by_keys(source, &["enrollment"]).ok_or_else(|| {
        KSeFError::StatusQueryFailed("certificate payload missing enrollment section".to_string())
    })?;

    let certificate_limit = u32_by_keys(certificate, &["limit"], "certificate.limit")?;
    let certificate_remaining = u32_by_keys(certificate, &["remaining"], "certificate.remaining")?;
    let enrollment_limit = u32_by_keys(enrollment, &["limit"], "enrollment.limit")?;
    let enrollment_remaining = u32_by_keys(enrollment, &["remaining"], "enrollment.remaining")?;

    Ok(CertificateLimits {
        max_active: certificate_limit,
        active: certificate_limit.saturating_sub(certificate_remaining),
        pending: enrollment_limit.saturating_sub(enrollment_remaining),
    })
}

#[async_trait]
impl KSeFCertificates for HttpKSeFCertificates {
    async fn get_limits(&self, access_token: &AccessToken) -> Result<CertificateLimits, KSeFError> {
        let url = format!("{}/certificates/limits", self.http.base_url);
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
            KSeFError::StatusQueryFailed(format!("parse certificate limits response: {e}"))
        })?;

        parse_limits(&payload)
    }

    async fn submit_enrollment(
        &self,
        access_token: &AccessToken,
        request: &CertificateEnrollmentRequest,
    ) -> Result<CertificateEnrollment, KSeFError> {
        let url = format!("{}/certificates/enrollments", self.http.base_url);
        let body = serde_json::json!({
            "certificateType": request.certificate_type.to_string(),
            "csr": request.csr_pem,
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
            KSeFError::StatusQueryFailed(format!("parse certificate enrollment response: {e}"))
        })?;

        parse_enrollment_from_payload(&payload)
    }

    async fn get_enrollment_status(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<CertificateEnrollment, KSeFError> {
        let url = format!(
            "{}/certificates/enrollments/{reference_number}",
            self.http.base_url
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
            KSeFError::StatusQueryFailed(format!("parse certificate status response: {e}"))
        })?;

        parse_enrollment_from_payload(&payload)
    }

    async fn query_certificates(
        &self,
        access_token: &AccessToken,
        request: &CertificateQueryRequest,
    ) -> Result<Vec<CertificateEnrollment>, KSeFError> {
        let mut url = reqwest::Url::parse(&format!("{}/certificates/query", self.http.base_url))
            .map_err(|e| KSeFError::StatusQueryFailed(format!("invalid certificates URL: {e}")))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(limit) = request.limit {
                query.append_pair("pageSize", &limit.to_string());
            }
            if let Some(offset) = request.offset {
                query.append_pair("pageOffset", &offset.to_string());
            }
        }
        let body = serde_json::json!({
            "status": request.status.map(|s| match s {
                EnrollmentStatus::Pending => "Pending",
                EnrollmentStatus::Approved => "Active",
                EnrollmentStatus::Rejected => "Rejected",
                EnrollmentStatus::Revoked => "Revoked",
            }),
        });

        let response = self
            .http
            .send(RateLimitCategory::Query, || {
                self.http
                    .client
                    .post(url.clone())
                    .bearer_auth(access_token.as_str())
                    .json(&body)
                    .send()
            })
            .await?;

        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse certificates query response: {e}"))
        })?;

        parse_enrollment_list(&payload)
    }

    async fn revoke_certificate(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<(), KSeFError> {
        let url = format!("{}/certificates/{reference_number}", self.http.base_url);
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
    fn parse_limits_accepts_valid_payload() {
        let payload = serde_json::json!({
            "canRequest": true,
            "certificate": { "limit": 10, "remaining": 7 },
            "enrollment": { "limit": 12, "remaining": 10 }
        });

        let parsed = parse_limits(&payload).unwrap();
        assert_eq!(parsed.max_active, 10);
        assert_eq!(parsed.active, 3);
        assert_eq!(parsed.pending, 2);
    }

    #[test]
    fn parse_enrollment_item_rejects_missing_fields() {
        let payload = serde_json::json!({
            "referenceNumber": "cert-1",
            "certificateType": "seal",
            "submittedAt": "2026-04-13T10:00:00Z",
            "updatedAt": "2026-04-13T10:00:00Z"
        });

        let err = parse_enrollment_item(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }
}
