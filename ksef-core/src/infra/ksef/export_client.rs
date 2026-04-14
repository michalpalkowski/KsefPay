use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::export::{ExportJob, ExportStatus};
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_export::{ExportRequest, KSeFExport};

use super::http_base::{KSeFHttpClient, parse_public_keys, str_by_keys, value_by_keys};

pub struct HttpKSeFExport {
    http: KSeFHttpClient,
}

impl HttpKSeFExport {
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

    /// Generate AES key + IV, RSA-encrypt the key, return all material.
    async fn prepare_export_encryption(
        &self,
        access_token: &AccessToken,
    ) -> Result<(String, String, Vec<u8>, Vec<u8>), KSeFError> {
        use openssl::encrypt::Encrypter;
        use openssl::hash::MessageDigest;
        use openssl::pkey::PKey;
        use openssl::rsa::{Padding, Rsa};
        use rand::RngCore;

        let url = format!("{}/security/public-key-certificates", self.http.base_url);
        let response = self
            .http
            .send(RateLimitCategory::PublicKey, || {
                self.http
                    .client
                    .get(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;
        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::PublicKeyFetchFailed(format!("parse public key response: {e}"))
        })?;
        let keys = parse_public_keys(&payload)?;
        let key = keys.first().ok_or_else(|| {
            KSeFError::PublicKeyFetchFailed("KSeF returned empty public key list".to_string())
        })?;

        // Generate random AES-256 key and IV
        let mut raw_aes_key = vec![0u8; 32];
        let mut raw_iv = vec![0u8; 16];
        rand::thread_rng().fill_bytes(&mut raw_aes_key);
        rand::thread_rng().fill_bytes(&mut raw_iv);

        // RSA-OAEP encrypt the AES key
        let rsa = Rsa::public_key_from_pem(key.pem().as_bytes())
            .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("invalid public key PEM: {e}")))?;
        let pkey = PKey::from_rsa(rsa)
            .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("PKey from RSA: {e}")))?;
        let mut encrypter = Encrypter::new(&pkey)
            .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("create encrypter: {e}")))?;
        encrypter.set_rsa_padding(Padding::PKCS1_OAEP).unwrap();
        encrypter.set_rsa_oaep_md(MessageDigest::sha256()).unwrap();
        encrypter.set_rsa_mgf1_md(MessageDigest::sha256()).unwrap();
        let buf_len = encrypter.encrypt_len(&raw_aes_key).unwrap();
        let mut encrypted_key = vec![0u8; buf_len];
        let len = encrypter
            .encrypt(&raw_aes_key, &mut encrypted_key)
            .map_err(|e| KSeFError::PublicKeyFetchFailed(format!("RSA encrypt AES key: {e}")))?;
        encrypted_key.truncate(len);

        Ok((
            openssl::base64::encode_block(&encrypted_key),
            openssl::base64::encode_block(&raw_iv),
            raw_aes_key,
            raw_iv,
        ))
    }
}

fn parse_status(raw: &str) -> Result<ExportStatus, KSeFError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" | "processing" => Ok(ExportStatus::Pending),
        "completed" | "done" => Ok(ExportStatus::Completed),
        "failed" | "error" => Ok(ExportStatus::Failed),
        _ => Err(KSeFError::StatusQueryFailed(format!(
            "invalid export status: '{raw}'"
        ))),
    }
}

fn parse_status_code(code: i64) -> ExportStatus {
    match code {
        100 => ExportStatus::Pending,
        200 => ExportStatus::Completed,
        _ => ExportStatus::Failed,
    }
}

fn parse_export_job_with_reference(
    payload: &Value,
    reference_fallback: Option<&str>,
) -> Result<ExportJob, KSeFError> {
    let source = if payload.is_object() {
        value_by_keys(payload, &["export", "item", "result"]).unwrap_or(payload)
    } else {
        payload
    };

    let reference_number = str_by_keys(source, &["referenceNumber", "reference", "id"])
        .map(str::to_string)
        .or_else(|| reference_fallback.map(str::to_string))
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "export response missing referenceNumber/reference/id".to_string(),
            )
        })?;
    let status = if let Some(status_raw) = str_by_keys(source, &["status", "exportStatus"]) {
        parse_status(status_raw)?
    } else if let Some(code) = source
        .get("status")
        .and_then(|status| status.get("code"))
        .and_then(Value::as_i64)
    {
        parse_status_code(code)
    } else {
        ExportStatus::Pending
    };
    let download_url = str_by_keys(source, &["downloadUrl", "url"])
        .map(str::to_string)
        .or_else(|| {
            source
                .get("package")
                .and_then(|p| p.get("parts"))
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| str_by_keys(part, &["url"]))
                .map(str::to_string)
        });
    let error_message = str_by_keys(source, &["errorMessage", "error"])
        .map(str::to_string)
        .or_else(|| {
            source
                .get("status")
                .and_then(|s| s.get("description"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    Ok(ExportJob {
        reference_number,
        status,
        download_url,
        error_message,
        encryption_key: None,
        encryption_iv: None,
    })
}

fn parse_export_job(payload: &Value) -> Result<ExportJob, KSeFError> {
    parse_export_job_with_reference(payload, None)
}

#[async_trait]
impl KSeFExport for HttpKSeFExport {
    async fn start_export(
        &self,
        access_token: &AccessToken,
        request: &ExportRequest,
    ) -> Result<ExportJob, KSeFError> {
        let url = format!("{}/invoices/exports", self.http.base_url);
        let (encrypted_symmetric_key, initialization_vector, raw_aes_key, raw_iv) =
            self.prepare_export_encryption(access_token).await?;
        let body = serde_json::json!({
            "encryption": {
                "encryptedSymmetricKey": encrypted_symmetric_key,
                "initializationVector": initialization_vector,
            },
            "onlyMetadata": false,
            "filters": {
                "subjectType": request.query.subject_type.api_value(),
                "dateRange": {
                    "dateType": "Invoicing",
                    "from": format!("{}T00:00:00Z", request.query.date_from),
                    "to": format!("{}T23:59:59Z", request.query.date_to),
                }
            }
        });

        let response = self
            .http
            .send(RateLimitCategory::Query, || {
                self.http
                    .client
                    .post(&url)
                    .bearer_auth(access_token.as_str())
                    .json(&body)
                    .send()
            })
            .await?;

        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse export-start response: {e}"))
        })?;
        let mut job = parse_export_job(&payload)?;
        job.encryption_key = Some(raw_aes_key);
        job.encryption_iv = Some(raw_iv);
        Ok(job)
    }

    async fn get_export_status(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<ExportJob, KSeFError> {
        let url = format!("{}/invoices/exports/{reference_number}", self.http.base_url);
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
            KSeFError::StatusQueryFailed(format!("parse export-status response: {e}"))
        })?;
        parse_export_job_with_reference(&payload, Some(reference_number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_export_job_accepts_valid_payload() {
        let payload = serde_json::json!({
            "referenceNumber": "exp-1",
            "status": "completed",
            "downloadUrl": "https://example.test/export.zip"
        });
        let parsed = parse_export_job(&payload).unwrap();
        assert_eq!(parsed.reference_number, "exp-1");
        assert!(matches!(parsed.status, ExportStatus::Completed));
        assert!(parsed.download_url.is_some());
    }

    #[test]
    fn parse_export_job_rejects_missing_reference() {
        let payload = serde_json::json!({"status": "pending"});
        let err = parse_export_job(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }
}
