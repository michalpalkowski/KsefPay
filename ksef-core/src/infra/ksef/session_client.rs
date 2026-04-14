use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::crypto::{EncryptedInvoice, KSeFPublicKey};
use crate::domain::environment::KSeFEnvironment;
use crate::domain::session::{InvoiceMetadata, InvoiceQuery, KSeFNumber, SessionReference, Upo};
use crate::domain::xml::UntrustedInvoiceXml;
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_client::KSeFClient;

use super::http_base::{KSeFHttpClient, parse_public_keys, str_by_keys, value_by_keys};

/// HTTP implementation of `KSeFClient` using `reqwest`.
pub struct HttpKSeFClient {
    http: KSeFHttpClient,
}

impl HttpKSeFClient {
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

#[derive(Deserialize)]
struct OpenSessionResponse {
    #[serde(alias = "referenceNumber")]
    reference_number: String,
}

#[derive(Deserialize)]
struct SendInvoiceResponse {
    #[serde(alias = "elementReferenceNumber")]
    element_ref: Option<String>,
    #[serde(alias = "ksefReferenceNumber")]
    ksef_ref: Option<String>,
    #[serde(alias = "referenceNumber")]
    reference: Option<String>,
    #[serde(alias = "invoiceReferenceNumber")]
    invoice_ref: Option<String>,
}

#[derive(Deserialize)]
struct CloseSessionResponse {
    #[serde(alias = "referenceNumber")]
    reference_number: String,
    upo: Option<String>,
    #[serde(alias = "upoReferenceNumber")]
    upo_reference_number: Option<String>,
}

fn parse_date(raw: &str) -> Option<chrono::NaiveDate> {
    if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Some(date);
    }
    raw.get(0..10)
        .and_then(|prefix| chrono::NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok())
}

fn metadata_items(payload: &Value) -> Option<&Vec<Value>> {
    payload.as_array().or_else(|| {
        value_by_keys(
            payload,
            &[
                "items",
                "invoices",
                "invoiceMetadataList",
                "invoiceHeaderList",
                "result",
            ],
        )
        .and_then(Value::as_array)
    })
}

fn parse_metadata(payload: &Value) -> Result<Vec<InvoiceMetadata>, KSeFError> {
    let Some(items) = metadata_items(payload) else {
        return Err(KSeFError::StatusQueryFailed(
            "unexpected query_invoices response format".to_string(),
        ));
    };

    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        let ksef_number = str_by_keys(
            item,
            &[
                "ksefReferenceNumber",
                "ksefNumber",
                "ksef_number",
                "elementReferenceNumber",
            ],
        )
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "query_invoices item missing KSeF reference number".to_string(),
            )
        })?;

        let date_raw = str_by_keys(
            item,
            &[
                "invoiceDate",
                "date",
                "issueDate",
                "faDate",
                "acquisitionDate",
            ],
        )
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed("query_invoices item missing invoice date".to_string())
        })?;
        let invoice_date = parse_date(date_raw).ok_or_else(|| {
            KSeFError::StatusQueryFailed(format!(
                "query_invoices item has invalid invoice date: {date_raw}"
            ))
        })?;

        let subject_nip = str_by_keys(item, &["subjectNip", "sellerNip", "nip", "taxNumber"])
            .or_else(|| {
                item.get("seller")
                    .and_then(|s| s.get("nip"))
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "query_invoices item missing subject NIP/identifier".to_string(),
                )
            })?
            .to_string();

        parsed.push(InvoiceMetadata {
            ksef_number: KSeFNumber::new(ksef_number.to_string()),
            subject_nip,
            invoice_date,
        });
    }
    Ok(parsed)
}

#[async_trait]
impl KSeFClient for HttpKSeFClient {
    async fn open_session(
        &self,
        access_token: &AccessToken,
        session_encryption: &EncryptedInvoice,
    ) -> Result<SessionReference, KSeFError> {
        let url = format!("{}/sessions/online", self.http.base_url);

        let encrypted_symmetric_key = openssl::base64::encode_block(session_encryption.aes_key());
        let initialization_vector = openssl::base64::encode_block(session_encryption.iv());
        let body = serde_json::json!({
            "formCode": {
                "systemCode": "FA (3)",
                "schemaVersion": "1-0E",
                "value": "FA"
            },
            "encryption": {
                "encryptedSymmetricKey": encrypted_symmetric_key,
                "initializationVector": initialization_vector
            }
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

        let data: OpenSessionResponse = response
            .json()
            .await
            .map_err(|e| KSeFError::SessionOpenFailed(format!("parse response: {e}")))?;

        Ok(SessionReference::new(data.reference_number))
    }

    async fn send_invoice(
        &self,
        access_token: &AccessToken,
        session: &SessionReference,
        encrypted_invoice: &EncryptedInvoice,
    ) -> Result<KSeFNumber, KSeFError> {
        let url = format!(
            "{}/sessions/online/{}/invoices",
            self.http.base_url, session
        );

        let encrypted_invoice_content = openssl::base64::encode_block(encrypted_invoice.data());

        let body = serde_json::json!({
            "invoiceHash": encrypted_invoice.plaintext_hash_sha256_base64(),
            "invoiceSize": encrypted_invoice.plaintext_size_bytes(),
            "encryptedInvoiceHash": encrypted_invoice.encrypted_hash_sha256_base64(),
            "encryptedInvoiceSize": encrypted_invoice.encrypted_size_bytes(),
            "encryptedInvoiceContent": encrypted_invoice_content
        });

        let response = self
            .http
            .send(RateLimitCategory::Invoice, || {
                self.http
                    .client
                    .post(&url)
                    .bearer_auth(access_token.as_str())
                    .json(&body)
                    .send()
            })
            .await?;

        let body = response.text().await.map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("failed to read send_invoice body: {e}"))
        })?;
        let data: SendInvoiceResponse = serde_json::from_str(&body).map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("parse response: {e}; body={body}"))
        })?;

        let ksef_number = data
            .ksef_ref
            .or(data.element_ref)
            .or(data.reference)
            .or(data.invoice_ref)
            .ok_or_else(|| {
                KSeFError::InvoiceSubmissionFailed("no reference number in response".to_string())
            })?;

        Ok(KSeFNumber::new(ksef_number))
    }

    async fn close_session(
        &self,
        access_token: &AccessToken,
        session: &SessionReference,
    ) -> Result<Upo, KSeFError> {
        let url = format!("{}/sessions/online/{}/close", self.http.base_url, session);

        let response = self
            .http
            .send(RateLimitCategory::Session, || {
                self.http
                    .client
                    .post(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;

        let body = response.text().await.map_err(|e| {
            KSeFError::SessionCloseFailed(format!("failed to read close_session body: {e}"))
        })?;
        if body.trim().is_empty() {
            return Ok(Upo {
                reference: session.to_string(),
                content: Vec::new(),
            });
        }

        let data: CloseSessionResponse = serde_json::from_str(&body).map_err(|e| {
            KSeFError::SessionCloseFailed(format!("parse response: {e}; body={body}"))
        })?;

        let upo_ref = data.reference_number;
        if let Some(upo_payload) = data.upo {
            return Ok(Upo {
                reference: upo_ref,
                content: upo_payload.into_bytes(),
            });
        }
        if let Some(upo_reference_number) = data.upo_reference_number {
            return Ok(Upo {
                reference: upo_reference_number,
                content: Vec::new(),
            });
        }
        Ok(Upo {
            reference: upo_ref,
            content: body.into_bytes(),
        })
    }

    async fn get_upo(
        &self,
        access_token: &AccessToken,
        session: &SessionReference,
    ) -> Result<Upo, KSeFError> {
        let url = format!("{}/sessions/{}/upo", self.http.base_url, session);

        let response = self
            .http
            .send(RateLimitCategory::Session, || {
                self.http
                    .client
                    .get(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;

        let bytes = response.bytes().await?;
        Ok(Upo {
            reference: session.to_string(),
            content: bytes.to_vec(),
        })
    }

    async fn fetch_invoice(
        &self,
        access_token: &AccessToken,
        ksef_number: &KSeFNumber,
    ) -> Result<UntrustedInvoiceXml, KSeFError> {
        let url = format!("{}/invoices/ksef/{}", self.http.base_url, ksef_number);

        let response = self
            .http
            .send(RateLimitCategory::Invoice, || {
                self.http
                    .client
                    .get(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
            })
            .await?;

        let xml = response.text().await?;
        Ok(UntrustedInvoiceXml::new(xml))
    }

    async fn query_invoices(
        &self,
        access_token: &AccessToken,
        criteria: &InvoiceQuery,
    ) -> Result<Vec<InvoiceMetadata>, KSeFError> {
        let mut all_invoices = Vec::new();
        let mut page_offset: u32 = 0;
        let page_size: u32 = 100;

        loop {
            let url = format!(
                "{}/invoices/query/metadata?pageSize={page_size}&pageOffset={page_offset}",
                self.http.base_url
            );

            let body = serde_json::json!({
                "subjectType": criteria.subject_type.api_value(),
                "dateRange": {
                    "dateType": "Issue",
                    "from": format!("{}T00:00:00+00:00", criteria.date_from),
                    "to": format!("{}T23:59:59+00:00", criteria.date_to)
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

            let payload: Value = response
                .json()
                .await
                .map_err(|e| KSeFError::StatusQueryFailed(format!("parse response: {e}")))?;

            let page = parse_metadata(&payload)?;
            all_invoices.extend(page);

            let has_more = payload
                .get("hasMore")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if !has_more {
                break;
            }

            let is_truncated = payload
                .get("isTruncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if is_truncated {
                return Err(KSeFError::StatusQueryFailed(
                    "query result truncated at 10000 records — narrow the date range".to_string(),
                ));
            }

            page_offset += 1;
        }

        Ok(all_invoices)
    }

    async fn fetch_public_keys(&self) -> Result<Vec<KSeFPublicKey>, KSeFError> {
        let url = format!("{}/security/public-key-certificates", self.http.base_url);

        let response = self
            .http
            .send(RateLimitCategory::PublicKey, || {
                self.http.client.get(&url).send()
            })
            .await?;

        let body = response.text().await.map_err(|e| {
            KSeFError::PublicKeyFetchFailed(format!("failed to read public-key body: {e}"))
        })?;
        let payload: Value = serde_json::from_str(&body).map_err(|e| {
            KSeFError::PublicKeyFetchFailed(format!("parse response: {e}; body={body}"))
        })?;
        parse_public_keys(&payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_metadata_accepts_items_container() {
        let payload = serde_json::json!({
            "items": [
                {
                    "ksefReferenceNumber": "KSeF-123",
                    "subjectNip": "5260250274",
                    "invoiceDate": "2026-04-13"
                }
            ]
        });

        let parsed = parse_metadata(&payload).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].ksef_number.as_str(), "KSeF-123");
        assert_eq!(parsed[0].subject_nip, "5260250274");
        assert_eq!(
            parsed[0].invoice_date,
            chrono::NaiveDate::from_ymd_opt(2026, 4, 13).unwrap()
        );
    }

    #[test]
    fn parse_metadata_accepts_direct_array() {
        let payload = serde_json::json!([
            {
                "ksefNumber": "KSeF-456",
                "subjectNip": "5260250274",
                "issueDate": "2026-04-12T00:00:00Z"
            }
        ]);

        let parsed = parse_metadata(&payload).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].ksef_number.as_str(), "KSeF-456");
        assert_eq!(parsed[0].subject_nip, "5260250274");
        assert_eq!(
            parsed[0].invoice_date,
            chrono::NaiveDate::from_ymd_opt(2026, 4, 12).unwrap()
        );
    }
}
