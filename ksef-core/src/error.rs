use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid NIP '{value}': {reason}")]
    InvalidNip { value: String, reason: &'static str },

    #[error("invalid invoice status transition from {from} to {to}")]
    InvalidStatusTransition { from: String, to: String },

    #[error("invalid money amount: {0}")]
    InvalidAmount(String),

    #[error("invalid VAT rate: {0}")]
    InvalidVatRate(String),

    #[error("invalid {type_name}: '{value}'")]
    InvalidParse {
        type_name: &'static str,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("KSeF API error {status_code}: {description}")]
pub struct KSeFApiErrorDetail {
    pub status_code: u16,
    pub ksef_code: Option<String>,
    pub description: String,
    pub details: Vec<String>,
    pub reference_number: Option<String>,
    pub processing_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KSeFApiErrorParseError {
    #[error("invalid retry-after header: '{0}'")]
    InvalidRetryAfter(String),

    #[error("retry-after header is required for HTTP 429 responses")]
    MissingRetryAfter,

    #[error("invalid KSeF error JSON: {0}")]
    InvalidJson(String),

    #[error("missing required KSeF error description")]
    MissingDescription,
}

pub fn parse_ksef_error_response(
    status_code: u16,
    retry_after_header: Option<&str>,
    body: &str,
) -> Result<KSeFError, KSeFApiErrorParseError> {
    if status_code == 429 {
        let raw_retry_after =
            retry_after_header.ok_or(KSeFApiErrorParseError::MissingRetryAfter)?;
        let retry_after_secs = raw_retry_after
            .trim()
            .parse::<u64>()
            .map_err(|_| KSeFApiErrorParseError::InvalidRetryAfter(raw_retry_after.to_string()))?;
        return Ok(KSeFError::RateLimited {
            retry_after_ms: retry_after_secs.saturating_mul(1000),
        });
    }

    let payload: Value = serde_json::from_str(body)
        .map_err(|e| KSeFApiErrorParseError::InvalidJson(e.to_string()))?;

    let description = payload
        .get("status")
        .and_then(|status| status.get("description"))
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("exception")
                .and_then(|exception| exception.get("exceptionDetailList"))
                .and_then(Value::as_array)
                .and_then(|details| details.first())
                .and_then(|item| item.get("exceptionDescription"))
                .and_then(Value::as_str)
        })
        .ok_or(KSeFApiErrorParseError::MissingDescription)?
        .to_string();

    let ksef_code = payload
        .get("status")
        .and_then(|status| status.get("code"))
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .or_else(|| value.as_u64().map(|v| v.to_string()))
        })
        .or_else(|| {
            payload
                .get("exception")
                .and_then(|exception| exception.get("exceptionDetailList"))
                .and_then(Value::as_array)
                .and_then(|details| details.first())
                .and_then(|item| item.get("exceptionCode"))
                .and_then(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.as_u64().map(|v| v.to_string()))
                })
        });

    let details = payload
        .get("status")
        .and_then(|status| status.get("details"))
        .and_then(Value::as_array)
        .or_else(|| {
            payload
                .get("exception")
                .and_then(|exception| exception.get("exceptionDetailList"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("details"))
                .and_then(Value::as_array)
        })
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let reference_number = payload
        .get("status")
        .and_then(|status| status.get("referenceNumber"))
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("exception")
                .and_then(|exception| exception.get("serviceCode"))
                .and_then(Value::as_str)
        })
        .or_else(|| payload.get("referenceNumber").and_then(Value::as_str))
        .map(str::to_string);

    let processing_code = payload
        .get("status")
        .and_then(|status| status.get("processingCode"))
        .and_then(Value::as_u64)
        .or_else(|| payload.get("processingCode").and_then(Value::as_u64))
        .and_then(|code| u32::try_from(code).ok());

    Ok(KSeFError::ApiError(KSeFApiErrorDetail {
        status_code,
        ksef_code,
        description,
        details,
        reference_number,
        processing_code,
    }))
}

#[must_use]
pub fn map_ksef_error_response(
    status_code: u16,
    retry_after_header: Option<&str>,
    body: &str,
) -> KSeFError {
    match parse_ksef_error_response(status_code, retry_after_header, body) {
        Ok(err) => err,
        Err(parse_error) => KSeFError::ApiError(KSeFApiErrorDetail {
            status_code,
            ksef_code: None,
            description: format!("malformed KSeF error response: {parse_error}"),
            details: vec![body.to_string()],
            reference_number: None,
            processing_code: None,
        }),
    }
}

#[derive(Debug, Error)]
pub enum KSeFError {
    #[error("KSeF auth challenge failed: {0}")]
    ChallengeFailed(String),

    #[error("KSeF auth polling failed: {0}")]
    AuthPollingFailed(String),

    #[error("KSeF token redeem failed: {0}")]
    TokenRedeemFailed(String),

    #[error("KSeF token refresh failed: {0}")]
    TokenRefreshFailed(String),

    #[error("KSeF session open failed: {0}")]
    SessionOpenFailed(String),

    #[error("KSeF session close failed: {0}")]
    SessionCloseFailed(String),

    #[error("KSeF invoice submission failed: {0}")]
    InvoiceSubmissionFailed(String),

    #[error("KSeF invoice fetch failed: {0}")]
    InvoiceFetchFailed(String),

    #[error("KSeF status query failed: {0}")]
    StatusQueryFailed(String),

    #[error("KSeF public key fetch failed: {0}")]
    PublicKeyFetchFailed(String),

    #[error(transparent)]
    ApiError(KSeFApiErrorDetail),

    #[error("rate limit exceeded, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("KSeF HTTP error: {status} {body}")]
    HttpError { status: u16, body: String },

    #[error("KSeF request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("AES encryption failed: {0}")]
    AesEncryptionFailed(String),

    #[error("RSA encryption failed: {0}")]
    RsaEncryptionFailed(String),

    #[error("XAdES signing failed: {0}")]
    XadesSigningFailed(String),

    #[error("certificate generation failed: {0}")]
    CertificateGenerationFailed(String),

    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("entity not found: {entity} with id {id}")]
    NotFound { entity: &'static str, id: String },

    #[error("duplicate entity: {entity} with key {key}")]
    Duplicate { entity: &'static str, key: String },

    #[error("storage error: {0}")]
    Storage(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("job enqueue failed: {0}")]
    EnqueueFailed(String),

    #[error("job dequeue failed: {0}")]
    DequeueFailed(String),

    #[error("job not found: {0}")]
    JobNotFound(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Error)]
pub enum XmlError {
    #[error("XML serialization failed: {0}")]
    SerializationFailed(String),

    #[error("XML deserialization failed: {0}")]
    DeserializationFailed(String),

    #[error("XML validation failed: {0}")]
    ValidationFailed(String),

    #[error("XML parse error: {0}")]
    ParseFailed(String),

    #[error("missing required XML element: {0}")]
    MissingElement(String),

    #[error("invalid value in XML element '{element}': {reason}")]
    InvalidValue { element: String, reason: String },

    #[error("unsupported FA(3) schema version: {0}")]
    UnsupportedSchemaVersion(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_error_display_invalid_nip() {
        let err = DomainError::InvalidNip {
            value: "123".to_string(),
            reason: "must be exactly 10 digits",
        };
        assert_eq!(
            err.to_string(),
            "invalid NIP '123': must be exactly 10 digits"
        );
    }

    #[test]
    fn domain_error_display_invalid_status_transition() {
        let err = DomainError::InvalidStatusTransition {
            from: "accepted".to_string(),
            to: "draft".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "invalid invoice status transition from accepted to draft"
        );
    }

    #[test]
    fn repository_error_display_not_found() {
        let err = RepositoryError::NotFound {
            entity: "Invoice",
            id: "abc-123".to_string(),
        };
        assert_eq!(err.to_string(), "entity not found: Invoice with id abc-123");
    }

    #[test]
    fn repository_error_display_duplicate() {
        let err = RepositoryError::Duplicate {
            entity: "Invoice",
            key: "KSEF-123".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "duplicate entity: Invoice with key KSEF-123"
        );
    }

    #[test]
    fn repository_error_display_storage() {
        let err = RepositoryError::Storage("decrypt failed".to_string());
        assert_eq!(err.to_string(), "storage error: decrypt failed");
    }

    #[test]
    fn crypto_error_display() {
        let err = CryptoError::AesEncryptionFailed("bad padding".to_string());
        assert_eq!(err.to_string(), "AES encryption failed: bad padding");
    }

    #[test]
    fn queue_error_display() {
        let err = QueueError::JobNotFound("job-456".to_string());
        assert_eq!(err.to_string(), "job not found: job-456");
    }

    #[test]
    fn xml_error_display() {
        let err = XmlError::ValidationFailed("missing Naglowek element".to_string());
        assert_eq!(
            err.to_string(),
            "XML validation failed: missing Naglowek element"
        );
    }

    #[test]
    fn parse_ksef_error_response_parses_exception_payload() {
        let body = r#"{
          "exception": {
            "exceptionDetailList": [
              {
                "exceptionCode": 9105,
                "exceptionDescription": "Nieprawidlowy podpis",
                "details": ["Nieprawidlowa wartosc skrotu"]
              }
            ],
            "serviceCode": "00-service-ref"
          }
        }"#;

        let parsed = parse_ksef_error_response(400, None, body).unwrap();
        match parsed {
            KSeFError::ApiError(detail) => {
                assert_eq!(detail.status_code, 400);
                assert_eq!(detail.ksef_code.as_deref(), Some("9105"));
                assert_eq!(detail.description, "Nieprawidlowy podpis");
                assert_eq!(
                    detail.details,
                    vec!["Nieprawidlowa wartosc skrotu".to_string()]
                );
                assert_eq!(detail.reference_number.as_deref(), Some("00-service-ref"));
                assert_eq!(detail.processing_code, None);
            }
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    #[test]
    fn parse_ksef_error_response_missing_required_fields_returns_err() {
        let err = parse_ksef_error_response(400, None, "{}").unwrap_err();
        assert_eq!(err, KSeFApiErrorParseError::MissingDescription);
    }

    #[test]
    fn parse_ksef_error_response_429_with_retry_after_returns_rate_limited() {
        let parsed = parse_ksef_error_response(429, Some("7"), "{}").unwrap();
        match parsed {
            KSeFError::RateLimited { retry_after_ms } => assert_eq!(retry_after_ms, 7000),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }
}
