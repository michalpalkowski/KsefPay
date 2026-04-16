use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::domain::auth::AccessToken;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::permission::{
    PermissionGrantRequest, PermissionRecord, PermissionRevokeRequest, PermissionType,
};
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::{RateLimitCategory, TokenBucketRateLimiter};
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_permissions::{KSeFPermissions, PermissionQueryRequest};

use super::http_base::{KSeFHttpClient, parse_dt, str_by_keys, value_by_keys};

pub struct HttpKSeFPermissions {
    http: KSeFHttpClient,
}

impl HttpKSeFPermissions {
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

    async fn grant_permissions_persons_grants(
        &self,
        access_token: &AccessToken,
        request: &PermissionGrantRequest,
    ) -> Result<(), KSeFError> {
        let url = format!("{}/permissions/persons/grants", self.http.base_url);
        let request_nonce = Utc::now().timestamp_nanos_opt().map_or_else(
            || Utc::now().timestamp_millis().to_string(),
            |v| v.to_string(),
        );
        let permissions: Vec<String> = request
            .permissions
            .iter()
            .map(ToString::to_string)
            .collect();
        let body = serde_json::json!({
            "subjectIdentifier": {
                "type": "Nip",
                "value": request.authorized_nip.as_str(),
            },
            "permissions": permissions,
            "description": format!("fallback-grant-{request_nonce}"),
            "subjectDetails": {
                "subjectDetailsType": "PersonByIdentifier",
                "personById": {
                    "firstName": "E2E",
                    "lastName": "Test",
                }
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
        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse permissions grant response: {e}"))
        })?;
        let reference_number = str_by_keys(&payload, &["referenceNumber", "reference"])
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "permissions grant response missing referenceNumber/reference".to_string(),
                )
            })?
            .to_string();

        self.wait_for_permissions_operation(access_token, &reference_number)
            .await
    }

    async fn grant_permissions_entities_grants(
        &self,
        access_token: &AccessToken,
        request: &PermissionGrantRequest,
    ) -> Result<(), KSeFError> {
        let url = format!("{}/permissions/entities/grants", self.http.base_url);
        let request_nonce = Utc::now().timestamp_nanos_opt().map_or_else(
            || Utc::now().timestamp_millis().to_string(),
            |v| v.to_string(),
        );
        let permissions: Vec<Value> = request
            .permissions
            .iter()
            .map(|permission| {
                serde_json::json!({
                    "type": permission.to_string(),
                    "canDelegate": false,
                })
            })
            .collect();
        let body = serde_json::json!({
            "subjectIdentifier": {
                "type": "Nip",
                "value": request.authorized_nip.as_str(),
            },
            "permissions": permissions,
            "description": format!("fallback-entity-grant-{request_nonce}"),
            "subjectDetails": {
                "fullName": format!("Podmiot {}", request.authorized_nip),
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
        let payload: Value = response.json().await.map_err(|e| {
            KSeFError::StatusQueryFailed(format!("parse entity permissions grant response: {e}"))
        })?;
        let reference_number = str_by_keys(&payload, &["referenceNumber", "reference"])
            .ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "entity permissions grant response missing referenceNumber/reference"
                        .to_string(),
                )
            })?
            .to_string();

        self.wait_for_permissions_operation(access_token, &reference_number)
            .await
    }

    async fn wait_for_permissions_operation(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<(), KSeFError> {
        let url = format!(
            "{}/permissions/operations/{reference_number}",
            self.http.base_url
        );
        for _ in 0..20 {
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
                KSeFError::StatusQueryFailed(format!("parse permissions operation response: {e}"))
            })?;
            let status = payload
                .get("status")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    KSeFError::StatusQueryFailed(
                        "permissions operation response missing status object".to_string(),
                    )
                })?;
            let code = status.get("code").and_then(Value::as_i64).ok_or_else(|| {
                KSeFError::StatusQueryFailed(
                    "permissions operation response missing status.code".to_string(),
                )
            })?;
            let description = status
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("<missing description>");
            match code {
                100 => tokio::time::sleep(Duration::from_millis(1000)).await,
                200 => return Ok(()),
                other => {
                    return Err(KSeFError::StatusQueryFailed(format!(
                        "permissions operation {reference_number} failed with code {other}: {description}"
                    )));
                }
            }
        }

        Err(KSeFError::StatusQueryFailed(format!(
            "permissions operation {reference_number} timed out"
        )))
    }
}

fn parse_permission_type(raw: &str) -> Result<PermissionType, KSeFError> {
    raw.parse::<PermissionType>().map_err(|_| {
        KSeFError::StatusQueryFailed(format!("invalid permission type in response: '{raw}'"))
    })
}

fn parse_permission_records(payload: &Value) -> Result<Vec<PermissionRecord>, KSeFError> {
    let items = if let Some(array) = payload.as_array() {
        array
    } else if let Some(array) =
        value_by_keys(payload, &["permissions", "items", "result"]).and_then(Value::as_array)
    {
        array
    } else {
        return Err(KSeFError::StatusQueryFailed(
            "unexpected permissions response format".to_string(),
        ));
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let permission_raw = str_by_keys(
            item,
            &["permissionScope", "permissionType", "permission"],
        )
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "permission item missing permissionScope/permissionType/permission".to_string(),
            )
        })?;
        let granted_at_raw = str_by_keys(
            item,
            &["startDate", "grantedAt", "createdAt", "timestamp"],
        )
        .ok_or_else(|| {
            KSeFError::StatusQueryFailed(
                "permission item missing startDate/grantedAt/createdAt/timestamp".to_string(),
            )
        })?;

        let valid_to = str_by_keys(item, &["validTo", "expiresAt"])
            .map(|value| parse_dt(value, "validTo"))
            .transpose()?;

        out.push(PermissionRecord {
            permission: parse_permission_type(permission_raw)?,
            granted_at: parse_dt(granted_at_raw, "grantedAt")?,
            valid_to,
        });
    }

    Ok(out)
}

#[async_trait]
impl KSeFPermissions for HttpKSeFPermissions {
    async fn grant_permissions(
        &self,
        access_token: &AccessToken,
        request: &PermissionGrantRequest,
    ) -> Result<(), KSeFError> {
        let all_entity_permissions = request.permissions.iter().all(|permission| {
            matches!(
                permission,
                PermissionType::InvoiceRead | PermissionType::InvoiceWrite
            )
        });

        if all_entity_permissions {
            self.grant_permissions_entities_grants(access_token, request)
                .await
        } else {
            self.grant_permissions_persons_grants(access_token, request)
                .await
        }
    }

    async fn revoke_permissions(
        &self,
        access_token: &AccessToken,
        request: &PermissionRevokeRequest,
    ) -> Result<(), KSeFError> {
        let url = format!("{}/testdata/permissions/revoke", self.http.base_url);
        let permission_list: Vec<String> = request
            .permissions
            .iter()
            .map(ToString::to_string)
            .collect();
        let body = serde_json::json!({
            "contextIdentifier": {
                "type": "Nip",
                "value": request.context_nip.as_str(),
            },
            "authorizedIdentifier": {
                "type": "Nip",
                "value": request.authorized_nip.as_str(),
            },
            "permissions": permission_list,
        });

        self.http
            .send(RateLimitCategory::Session, || {
                self.http
                    .client
                    .post(&url)
                    .bearer_auth(access_token.as_str())
                    .json(&body)
                    .send()
            })
            .await?;

        Ok(())
    }

    async fn query_permissions(
        &self,
        access_token: &AccessToken,
        request: &PermissionQueryRequest,
    ) -> Result<Vec<PermissionRecord>, KSeFError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/permissions/query/persons/grants",
            self.http.base_url
        ))
        .map_err(|e| KSeFError::StatusQueryFailed(format!("invalid permissions URL: {e}")))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("pageOffset", "0");
            query.append_pair("pageSize", "100");
        }
        let body = serde_json::json!({
            "queryType": "PermissionsInCurrentContext",
            "authorizedIdentifier": request.authorized_nip.as_ref().map(|nip| serde_json::json!({
                "type": "Nip",
                "value": nip.as_str(),
            })),
            "permissionTypes": request.permission.map(|p| vec![p.to_string()]),
            "authorIdentifier": {
                "type": "Nip",
                "value": request.context_nip.as_str(),
            }
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
            KSeFError::StatusQueryFailed(format!("parse permissions response: {e}"))
        })?;
        parse_permission_records(&payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_permission_records_accepts_direct_array() {
        let payload = serde_json::json!([
            {
                "permissionType": "InvoiceRead",
                "grantedAt": "2026-04-13T10:00:00Z",
                "validTo": "2026-05-13T10:00:00Z"
            },
            {
                "permissionType": "CredentialsManage",
                "grantedAt": "2026-04-13T10:00:01Z"
            }
        ]);

        let parsed = parse_permission_records(&payload).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].permission, PermissionType::InvoiceRead);
        assert!(parsed[0].valid_to.is_some());
        assert_eq!(parsed[1].permission, PermissionType::CredentialsManage);
        assert!(parsed[1].valid_to.is_none());
    }

    #[test]
    fn parse_permission_records_rejects_missing_fields() {
        let payload = serde_json::json!([{ "grantedAt": "2026-04-13T10:00:00Z" }]);
        let err = parse_permission_records(&payload).unwrap_err();
        assert!(matches!(err, KSeFError::StatusQueryFailed(_)));
    }
}
