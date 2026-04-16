//! `KSeF` sandbox test data management.
//!
//! On the test environment (`api-test.ksef.mf.gov.pl`) subjects must be
//! registered via `/testdata/*` endpoints before they can authenticate.
//! This module provides Rust functions for that setup.

use reqwest::Client;
use serde::Deserialize;

use crate::domain::environment::KSeFEnvironment;
use crate::domain::nip::Nip;
use crate::error::KSeFError;

/// Client for `KSeF` test data management endpoints (`/testdata/*`).
///
/// Only works on test and demo environments. Production does not
/// expose these endpoints.
pub struct TestDataClient {
    client: Client,
    base_url: String,
}

/// Result of creating a test subject.
#[derive(Debug)]
pub enum SubjectCreateResult {
    /// Subject was created successfully.
    Created,
    /// Subject already exists (code 30001) — not an error.
    AlreadyExists,
}

/// Result of granting permissions.
#[derive(Debug)]
pub enum PermissionsGrantResult {
    /// Permissions granted successfully.
    Granted,
    /// Permissions already exist.
    AlreadyExist,
    /// `KSeF` returned HTTP 500 — known sandbox bug.
    SandboxError(String),
}

/// Allowed subject types for test data creation.
#[derive(Debug, Clone, Copy)]
pub enum TestSubjectType {
    EnforcementAuthority,
    VatGroup,
    Jst,
}

impl TestSubjectType {
    fn as_str(self) -> &'static str {
        match self {
            Self::EnforcementAuthority => "EnforcementAuthority",
            Self::VatGroup => "VatGroup",
            Self::Jst => "JST",
        }
    }
}

/// Allowed permission types.
#[derive(Debug, Clone, Copy)]
pub enum TestPermissionType {
    InvoiceRead,
    InvoiceWrite,
    Introspection,
    CredentialsRead,
    CredentialsManage,
    EnforcementOperations,
    SubunitManage,
}

impl TestPermissionType {
    fn as_str(self) -> &'static str {
        match self {
            Self::InvoiceRead => "InvoiceRead",
            Self::InvoiceWrite => "InvoiceWrite",
            Self::Introspection => "Introspection",
            Self::CredentialsRead => "CredentialsRead",
            Self::CredentialsManage => "CredentialsManage",
            Self::EnforcementOperations => "EnforcementOperations",
            Self::SubunitManage => "SubunitManage",
        }
    }
}

async fn read_error_body(response: reqwest::Response) -> String {
    match response.text().await {
        Ok(body) => body,
        Err(err) => format!("<failed to read response body: {err}>"),
    }
}

#[derive(Deserialize)]
struct ExceptionResponse {
    exception: Option<ExceptionDetail>,
}

#[derive(Deserialize)]
struct ExceptionDetail {
    #[serde(alias = "exceptionDetailList", default)]
    details: Vec<ExceptionItem>,
}

#[derive(Deserialize)]
struct ExceptionItem {
    #[serde(alias = "exceptionCode")]
    code: u32,
    #[serde(alias = "exceptionDescription")]
    #[allow(dead_code)]
    description: String,
}

fn is_already_exists_error(body: &str) -> bool {
    serde_json::from_str::<ExceptionResponse>(body)
        .ok()
        .and_then(|r| r.exception)
        .is_some_and(|d| d.details.iter().any(|item| item.code == 30001))
}

impl TestDataClient {
    /// Create a new test data client.
    ///
    /// Only test and demo environments are supported. Panics on production.
    #[must_use]
    pub fn new(environment: KSeFEnvironment) -> Self {
        assert_ne!(
            environment,
            KSeFEnvironment::Production,
            "TestDataClient cannot be used on production"
        );
        Self {
            client: Client::new(),
            base_url: environment.api_base_url().to_string(),
        }
    }

    /// Register a test subject (company) on the sandbox.
    ///
    /// Idempotent — returns `AlreadyExists` if the NIP is already registered.
    pub async fn create_subject(
        &self,
        nip: &Nip,
        description: &str,
        subject_type: TestSubjectType,
    ) -> Result<SubjectCreateResult, KSeFError> {
        let url = format!("{}/testdata/subject", self.base_url);

        let body = serde_json::json!({
            "subjectNip": nip.as_str(),
            "subjectType": subject_type.as_str(),
            "description": description,
        });

        let response = self.client.post(&url).json(&body).send().await?;
        let status = response.status();

        if status.is_success() {
            return Ok(SubjectCreateResult::Created);
        }

        let body_text = read_error_body(response).await;
        if is_already_exists_error(&body_text) {
            return Ok(SubjectCreateResult::AlreadyExists);
        }

        Err(KSeFError::HttpError {
            status: status.as_u16(),
            body: body_text,
        })
    }

    /// Grant permissions to a subject on the sandbox.
    ///
    /// Known issue: this endpoint often returns HTTP 500 on the sandbox.
    pub async fn grant_permissions(
        &self,
        context_nip: &Nip,
        authorized_nip: &Nip,
        permissions: &[TestPermissionType],
    ) -> Result<PermissionsGrantResult, KSeFError> {
        let url = format!("{}/testdata/permissions", self.base_url);

        let perm_list: Vec<serde_json::Value> = permissions
            .iter()
            .map(|p| {
                serde_json::json!({
                    "description": format!("{} permission", p.as_str()),
                    "permissionType": p.as_str(),
                })
            })
            .collect();

        let body = serde_json::json!({
            "contextIdentifier": { "type": "Nip", "value": context_nip.as_str() },
            "authorizedIdentifier": { "type": "Nip", "value": authorized_nip.as_str() },
            "permissions": perm_list,
        });

        let response = self.client.post(&url).json(&body).send().await?;
        let status = response.status();

        if status.is_success() {
            return Ok(PermissionsGrantResult::Granted);
        }

        let body_text = read_error_body(response).await;

        // HTTP 500 is a known sandbox bug — report but don't fail hard
        if status.as_u16() == 500 {
            return Ok(PermissionsGrantResult::SandboxError(body_text));
        }

        if is_already_exists_error(&body_text) {
            return Ok(PermissionsGrantResult::AlreadyExist);
        }

        Err(KSeFError::HttpError {
            status: status.as_u16(),
            body: body_text,
        })
    }

    /// Convenience: register sandbox subject only.
    ///
    /// Owner access in the same NIP context is implicit, so this helper
    /// does not attempt self-grants.
    pub async fn setup_test_subject(&self, nip: &Nip) -> Result<SubjectCreateResult, KSeFError> {
        self.create_subject(
            nip,
            &format!("ksef-paymoney test subject NIP {nip}"),
            TestSubjectType::EnforcementAuthority,
        )
        .await
    }
}
