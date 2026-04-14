use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::batch::{BatchSession, BatchSessionStatus, PartUploadRequest, UploadUrl};
use crate::domain::environment::KSeFEnvironment;
use crate::domain::xml::InvoiceXml;
use crate::error::KSeFError;
use crate::infra::crypto::AesCbcEncryptor;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::encryption::InvoiceEncryptor;
use crate::ports::ksef_batch::{BatchOpenRequest, KSeFBatch};

use super::http_base::{KSeFHttpClient, parse_public_keys, str_by_keys, value_by_keys};

pub struct HttpKSeFBatch {
    http: KSeFHttpClient,
    part_upload_targets: Mutex<HashMap<String, HashMap<u32, UploadTarget>>>,
}

#[derive(Debug, Clone)]
struct UploadTarget {
    url: UploadUrl,
    headers: HashMap<String, String>,
}

impl HttpKSeFBatch {
    #[must_use]
    pub fn new(environment: KSeFEnvironment) -> Self {
        Self {
            http: KSeFHttpClient::new(environment),
            part_upload_targets: Mutex::new(HashMap::new()),
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
            part_upload_targets: Mutex::new(HashMap::new()),
        }
    }

    async fn prepare_batch_encryption(
        &self,
        access_token: &AccessToken,
    ) -> Result<(String, String), KSeFError> {
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

        let encryptor = AesCbcEncryptor;
        let encrypted = encryptor
            .encrypt(&InvoiceXml::new("<BatchSeed/>".to_string()), key)
            .await
            .map_err(|e| {
                KSeFError::InvoiceSubmissionFailed(format!("prepare batch encryption failed: {e}"))
            })?;

        Ok((
            openssl::base64::encode_block(encrypted.aes_key()),
            openssl::base64::encode_block(encrypted.iv()),
        ))
    }
}

fn parse_status(raw: &str) -> Result<BatchSessionStatus, KSeFError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "created" => Ok(BatchSessionStatus::Created),
        "uploading" => Ok(BatchSessionStatus::Uploading),
        "uploaded" => Ok(BatchSessionStatus::Uploaded),
        "processing" => Ok(BatchSessionStatus::Processing),
        "completed" => Ok(BatchSessionStatus::Completed),
        "failed" => Ok(BatchSessionStatus::Failed),
        "closed" => Ok(BatchSessionStatus::Closed),
        _ => Err(KSeFError::StatusQueryFailed(format!(
            "invalid batch status: '{raw}'"
        ))),
    }
}

fn parse_status_code(code: i64) -> BatchSessionStatus {
    match code {
        100 => BatchSessionStatus::Created,
        150 => BatchSessionStatus::Processing,
        170 | 440 => BatchSessionStatus::Closed,
        200 => BatchSessionStatus::Completed,
        _ => BatchSessionStatus::Failed,
    }
}

fn parse_batch_session_with_reference(
    payload: &Value,
    reference_fallback: Option<&str>,
) -> Result<BatchSession, KSeFError> {
    let source = if payload.is_object() {
        value_by_keys(payload, &["session", "item", "result"]).unwrap_or(payload)
    } else {
        payload
    };

    let reference_number = str_by_keys(source, &["referenceNumber", "reference", "id"])
        .map(str::to_string)
        .or_else(|| reference_fallback.map(str::to_string))
        .ok_or_else(|| KSeFError::StatusQueryFailed("missing batch reference".to_string()))?;
    let status = if let Some(status_raw) = str_by_keys(source, &["status"]) {
        parse_status(status_raw)?
    } else if let Some(code) = source
        .get("status")
        .and_then(|status| status.get("code"))
        .and_then(Value::as_i64)
    {
        parse_status_code(code)
    } else {
        BatchSessionStatus::Created
    };

    Ok(BatchSession {
        reference_number,
        status,
        created_at: Utc::now(),
        files: Vec::new(),
    })
}

fn parse_batch_session(payload: &Value) -> Result<BatchSession, KSeFError> {
    parse_batch_session_with_reference(payload, None)
}

fn parse_open_batch_upload_targets(
    payload: &Value,
) -> Result<HashMap<u32, UploadTarget>, KSeFError> {
    let targets = payload
        .get("partUploadRequests")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "open batch response missing partUploadRequests".to_string(),
            )
        })?;

    let mut parsed = HashMap::with_capacity(targets.len());
    for target in targets {
        let ordinal_u64 = target
            .get("ordinalNumber")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed("partUploadRequest missing ordinalNumber".to_string())
            })?;
        let ordinal = u32::try_from(ordinal_u64).map_err(|_| {
            KSeFError::StatusQueryFailed(format!(
                "partUploadRequest ordinalNumber out of range: {ordinal_u64}"
            ))
        })?;
        let url = str_by_keys(target, &["url"]).ok_or_else(|| {
            KSeFError::StatusQueryFailed("partUploadRequest missing url".to_string())
        })?;
        let upload_url = url.parse::<UploadUrl>().map_err(|_| {
            KSeFError::StatusQueryFailed(format!("invalid part upload url: '{url}'"))
        })?;
        let headers = target
            .get("headers")
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        parsed.insert(
            ordinal,
            UploadTarget {
                url: upload_url,
                headers,
            },
        );
    }
    Ok(parsed)
}

fn build_open_batch_request_body(
    request: &BatchOpenRequest,
    encrypted_symmetric_key: &str,
    initialization_vector: &str,
) -> Value {
    let file_parts = request
        .parts
        .iter()
        .map(|part| {
            serde_json::json!({
                "ordinalNumber": part.part_number,
                "fileSize": part.size_bytes,
                "fileHash": part.hash_sha256_base64,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "formCode": {
            "systemCode": "FA (3)",
            "schemaVersion": "1-0E",
            "value": "FA",
        },
        "batchFile": {
            "fileSize": request.file.file_size_bytes,
            "fileHash": request.file.file_hash_sha256_base64,
            "fileParts": file_parts,
        },
        "encryption": {
            "encryptedSymmetricKey": encrypted_symmetric_key,
            "initializationVector": initialization_vector,
        },
        "offlineMode": false
    })
}

#[async_trait]
impl KSeFBatch for HttpKSeFBatch {
    async fn open_batch_session(
        &self,
        access_token: &AccessToken,
        request: &BatchOpenRequest,
    ) -> Result<BatchSession, KSeFError> {
        if request.parts.is_empty() {
            return Err(KSeFError::InvoiceSubmissionFailed(
                "batch open request must include at least one file part".to_string(),
            ));
        }

        let url = format!("{}/sessions/batch", self.http.base_url);
        let (encrypted_symmetric_key, initialization_vector) =
            self.prepare_batch_encryption(access_token).await?;
        let body = build_open_batch_request_body(
            request,
            &encrypted_symmetric_key,
            &initialization_vector,
        );
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
        let payload: Value = response
            .json()
            .await
            .map_err(|e| KSeFError::StatusQueryFailed(format!("parse open-batch response: {e}")))?;
        let session = parse_batch_session(&payload)?;
        let targets = parse_open_batch_upload_targets(&payload)?;
        self.part_upload_targets
            .lock()
            .unwrap()
            .insert(session.reference_number.clone(), targets);
        Ok(session)
    }

    async fn upload_part(
        &self,
        _access_token: &AccessToken,
        request: &PartUploadRequest,
        payload: &[u8],
    ) -> Result<(), KSeFError> {
        if payload.is_empty() {
            return Err(KSeFError::InvoiceSubmissionFailed(
                "batch upload payload cannot be empty".to_string(),
            ));
        }

        let (part_url, headers) = if let Some(upload_url) = &request.upload_url {
            (upload_url.to_string(), HashMap::new())
        } else {
            let guard = self.part_upload_targets.lock().unwrap();
            let session_targets = guard.get(&request.session_reference).ok_or_else(|| {
                KSeFError::StatusQueryFailed(format!(
                    "missing upload targets for batch session '{}'",
                    request.session_reference
                ))
            })?;
            let target = session_targets
                .get(&request.part.part_number)
                .ok_or_else(|| {
                    KSeFError::StatusQueryFailed(format!(
                        "missing upload target for part {}",
                        request.part.part_number
                    ))
                })?;
            (target.url.to_string(), target.headers.clone())
        };

        self.http
            .send(RateLimitCategory::Invoice, || {
                let mut builder = self.http.client.put(&part_url).body(payload.to_vec());
                for (name, value) in &headers {
                    builder = builder.header(name, value);
                }
                builder.send()
            })
            .await?;
        Ok(())
    }

    async fn close_batch_session(
        &self,
        access_token: &AccessToken,
        session_reference: &str,
    ) -> Result<BatchSession, KSeFError> {
        let url = format!(
            "{}/sessions/batch/{session_reference}/close",
            self.http.base_url
        );
        self.http
            .send(RateLimitCategory::Session, || {
                self.http
                    .client
                    .post(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;
        Ok(BatchSession {
            reference_number: session_reference.to_string(),
            status: BatchSessionStatus::Closed,
            created_at: Utc::now(),
            files: Vec::new(),
        })
    }

    async fn get_batch_status(
        &self,
        access_token: &AccessToken,
        session_reference: &str,
    ) -> Result<BatchSession, KSeFError> {
        let url = format!("{}/sessions/{session_reference}", self.http.base_url);
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
            KSeFError::StatusQueryFailed(format!("parse batch-status response: {e}"))
        })?;
        parse_batch_session_with_reference(&payload, Some(session_reference))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::batch::{BatchFileInfo, BatchFilePartInfo};

    #[test]
    fn parse_batch_session_accepts_minimal_payload() {
        let payload = serde_json::json!({
            "referenceNumber": "batch-1",
            "status": "uploading"
        });
        let parsed = parse_batch_session(&payload).unwrap();
        assert_eq!(parsed.reference_number, "batch-1");
        assert!(matches!(parsed.status, BatchSessionStatus::Uploading));
    }

    #[test]
    fn parse_batch_session_rejects_missing_reference() {
        let payload = serde_json::json!({"status": "created"});
        let err = parse_batch_session(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }

    #[test]
    fn open_batch_body_maps_all_parts_contract() {
        let request = BatchOpenRequest {
            file: BatchFileInfo {
                file_name: "batch.zip".to_string(),
                file_size_bytes: 1234,
                file_hash_sha256_base64: "filehash".to_string(),
            },
            parts: vec![
                BatchFilePartInfo {
                    part_number: 1,
                    offset_bytes: 0,
                    size_bytes: 600,
                    hash_sha256_base64: "hash-1".to_string(),
                },
                BatchFilePartInfo {
                    part_number: 2,
                    offset_bytes: 600,
                    size_bytes: 634,
                    hash_sha256_base64: "hash-2".to_string(),
                },
            ],
        };

        let body = build_open_batch_request_body(&request, "enc-key", "iv");
        let parts = body
            .get("batchFile")
            .and_then(|v| v.get("fileParts"))
            .and_then(Value::as_array)
            .unwrap();

        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["ordinalNumber"], 1);
        assert_eq!(parts[0]["fileSize"], 600);
        assert_eq!(parts[0]["fileHash"], "hash-1");
        assert_eq!(parts[1]["ordinalNumber"], 2);
        assert_eq!(parts[1]["fileSize"], 634);
        assert_eq!(parts[1]["fileHash"], "hash-2");
    }
}
