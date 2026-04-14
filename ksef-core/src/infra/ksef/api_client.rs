use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::auth::{
    AccessToken, AuthChallenge, AuthReference, AuthSessionInfo, AuthStatus, ContextIdentifier,
    RefreshToken, TokenPair,
};
use crate::domain::batch::{BatchSession, PartUploadRequest};
use crate::domain::certificate_mgmt::{CertificateEnrollment, CertificateLimits};
use crate::domain::crypto::{EncryptedInvoice, KSeFPublicKey, SignedAuthRequest};
use crate::domain::environment::KSeFEnvironment;
use crate::domain::export::ExportJob;
use crate::domain::nip::Nip;
use crate::domain::permission::{
    PermissionGrantRequest, PermissionRecord, PermissionRevokeRequest,
};
use crate::domain::rate_limit::{ContextLimits, EffectiveApiRateLimits, SubjectLimits};
use crate::domain::session::{InvoiceMetadata, InvoiceQuery, KSeFNumber, SessionReference, Upo};
use crate::domain::token_mgmt::ManagedToken;
use crate::domain::xml::UntrustedInvoiceXml;
use crate::error::KSeFError;
use crate::infra::http::rate_limiter::TokenBucketRateLimiter;
use crate::infra::http::retry::RetryPolicy;
use crate::ports::ksef_auth::KSeFAuth;
use crate::ports::ksef_auth_sessions::KSeFAuthSessions;
use crate::ports::ksef_batch::{BatchOpenRequest, KSeFBatch};
use crate::ports::ksef_certificates::{
    CertificateEnrollmentRequest, CertificateQueryRequest, KSeFCertificates,
};
use crate::ports::ksef_client::KSeFClient;
use crate::ports::ksef_export::{ExportRequest, KSeFExport};
use crate::ports::ksef_peppol::{KSeFPeppol, PeppolProvidersResponse, PeppolQueryRequest};
use crate::ports::ksef_permissions::{KSeFPermissions, PermissionQueryRequest};
use crate::ports::ksef_rate_limits::KSeFRateLimits;
use crate::ports::ksef_tokens::{
    KSeFTokens, TokenGenerateRequest, TokenQueryRequest, TokenQueryResponse,
};

use super::auth_client::HttpKSeFAuth;
use super::auth_sessions_client::HttpKSeFAuthSessions;
use super::batch_client::HttpKSeFBatch;
use super::certificates_client::HttpKSeFCertificates;
use super::export_client::HttpKSeFExport;
use super::peppol_client::HttpKSeFPeppol;
use super::permissions_client::HttpKSeFPermissions;
use super::rate_limits_client::HttpKSeFRateLimits;
use super::session_client::HttpKSeFClient;
use super::tokens_client::HttpKSeFTokens;

/// Single entry point for all `KSeF` API communication.
///
/// Groups every HTTP client behind one struct so that:
/// - DI in `main.rs` creates one object instead of 10
/// - Services receive `Arc<KSeFApiClient>` instead of multiple `Arc<dyn Trait>`
/// - Rate limiter and retry policy are shared across all endpoints
pub struct KSeFApiClient {
    pub auth: HttpKSeFAuth,
    pub sessions: HttpKSeFAuthSessions,
    pub client: HttpKSeFClient,
    pub batch: HttpKSeFBatch,
    pub certificates: HttpKSeFCertificates,
    pub export: HttpKSeFExport,
    pub permissions: HttpKSeFPermissions,
    pub peppol: HttpKSeFPeppol,
    pub rate_limits: HttpKSeFRateLimits,
    pub tokens: HttpKSeFTokens,
}

impl KSeFApiClient {
    /// Create with default rate limiter and retry policy.
    #[must_use]
    pub fn new(environment: KSeFEnvironment) -> Self {
        Self::with_http_controls(
            environment,
            Arc::new(TokenBucketRateLimiter::default()),
            RetryPolicy::default(),
        )
    }

    /// Create with shared rate limiter and retry policy.
    #[must_use]
    pub fn with_http_controls(
        environment: KSeFEnvironment,
        rate_limiter: Arc<TokenBucketRateLimiter>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            auth: HttpKSeFAuth::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            sessions: HttpKSeFAuthSessions::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            client: HttpKSeFClient::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            batch: HttpKSeFBatch::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            certificates: HttpKSeFCertificates::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            export: HttpKSeFExport::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            permissions: HttpKSeFPermissions::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            peppol: HttpKSeFPeppol::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            rate_limits: HttpKSeFRateLimits::with_http_controls(
                environment,
                rate_limiter.clone(),
                retry_policy.clone(),
            ),
            tokens: HttpKSeFTokens::with_http_controls(environment, rate_limiter, retry_policy),
        }
    }
}

// ---------------------------------------------------------------------------
// Trait delegation — lets `Arc<KSeFApiClient>` be passed where services
// expect `Arc<dyn KSeFAuth>`, `Arc<dyn KSeFClient>`, etc.
// ---------------------------------------------------------------------------

#[async_trait]
impl KSeFAuth for KSeFApiClient {
    async fn request_challenge(&self, nip: &Nip) -> Result<AuthChallenge, KSeFError> {
        self.auth.request_challenge(nip).await
    }
    async fn authenticate_xades(
        &self,
        req: &SignedAuthRequest,
    ) -> Result<AuthReference, KSeFError> {
        self.auth.authenticate_xades(req).await
    }
    async fn authenticate_token(
        &self,
        ctx: &ContextIdentifier,
        token: &str,
    ) -> Result<AuthReference, KSeFError> {
        self.auth.authenticate_token(ctx, token).await
    }
    async fn poll_auth_status(&self, reference: &AuthReference) -> Result<AuthStatus, KSeFError> {
        self.auth.poll_auth_status(reference).await
    }
    async fn redeem_token(&self, reference: &AuthReference) -> Result<TokenPair, KSeFError> {
        self.auth.redeem_token(reference).await
    }
    async fn refresh_token(&self, refresh_token: &RefreshToken) -> Result<TokenPair, KSeFError> {
        self.auth.refresh_token(refresh_token).await
    }
}

#[async_trait]
impl KSeFClient for KSeFApiClient {
    async fn open_session(
        &self,
        token: &AccessToken,
        enc: &EncryptedInvoice,
    ) -> Result<SessionReference, KSeFError> {
        self.client.open_session(token, enc).await
    }
    async fn send_invoice(
        &self,
        token: &AccessToken,
        session: &SessionReference,
        enc: &EncryptedInvoice,
    ) -> Result<KSeFNumber, KSeFError> {
        self.client.send_invoice(token, session, enc).await
    }
    async fn close_session(
        &self,
        token: &AccessToken,
        session: &SessionReference,
    ) -> Result<Upo, KSeFError> {
        self.client.close_session(token, session).await
    }
    async fn get_upo(
        &self,
        token: &AccessToken,
        session: &SessionReference,
    ) -> Result<Upo, KSeFError> {
        self.client.get_upo(token, session).await
    }
    async fn fetch_invoice(
        &self,
        token: &AccessToken,
        ksef_number: &KSeFNumber,
    ) -> Result<UntrustedInvoiceXml, KSeFError> {
        self.client.fetch_invoice(token, ksef_number).await
    }
    async fn query_invoices(
        &self,
        token: &AccessToken,
        criteria: &InvoiceQuery,
    ) -> Result<Vec<InvoiceMetadata>, KSeFError> {
        self.client.query_invoices(token, criteria).await
    }
    async fn fetch_public_keys(&self) -> Result<Vec<KSeFPublicKey>, KSeFError> {
        self.client.fetch_public_keys().await
    }
}

#[async_trait]
impl KSeFBatch for KSeFApiClient {
    async fn open_batch_session(
        &self,
        token: &AccessToken,
        req: &BatchOpenRequest,
    ) -> Result<BatchSession, KSeFError> {
        self.batch.open_batch_session(token, req).await
    }
    async fn upload_part(
        &self,
        token: &AccessToken,
        req: &PartUploadRequest,
        payload: &[u8],
    ) -> Result<(), KSeFError> {
        self.batch.upload_part(token, req, payload).await
    }
    async fn close_batch_session(
        &self,
        token: &AccessToken,
        reference: &str,
    ) -> Result<BatchSession, KSeFError> {
        self.batch.close_batch_session(token, reference).await
    }
    async fn get_batch_status(
        &self,
        token: &AccessToken,
        reference: &str,
    ) -> Result<BatchSession, KSeFError> {
        self.batch.get_batch_status(token, reference).await
    }
}

#[async_trait]
impl KSeFExport for KSeFApiClient {
    async fn start_export(
        &self,
        token: &AccessToken,
        req: &ExportRequest,
    ) -> Result<ExportJob, KSeFError> {
        self.export.start_export(token, req).await
    }
    async fn get_export_status(
        &self,
        token: &AccessToken,
        reference: &str,
    ) -> Result<ExportJob, KSeFError> {
        self.export.get_export_status(token, reference).await
    }
}

#[async_trait]
impl KSeFPermissions for KSeFApiClient {
    async fn grant_permissions(
        &self,
        token: &AccessToken,
        req: &PermissionGrantRequest,
    ) -> Result<(), KSeFError> {
        self.permissions.grant_permissions(token, req).await
    }
    async fn revoke_permissions(
        &self,
        token: &AccessToken,
        req: &PermissionRevokeRequest,
    ) -> Result<(), KSeFError> {
        self.permissions.revoke_permissions(token, req).await
    }
    async fn query_permissions(
        &self,
        token: &AccessToken,
        req: &PermissionQueryRequest,
    ) -> Result<Vec<PermissionRecord>, KSeFError> {
        self.permissions.query_permissions(token, req).await
    }
}

#[async_trait]
impl KSeFTokens for KSeFApiClient {
    async fn generate_token(
        &self,
        token: &AccessToken,
        req: &TokenGenerateRequest,
    ) -> Result<ManagedToken, KSeFError> {
        self.tokens.generate_token(token, req).await
    }
    async fn query_tokens(
        &self,
        token: &AccessToken,
        req: &TokenQueryRequest,
    ) -> Result<TokenQueryResponse, KSeFError> {
        self.tokens.query_tokens(token, req).await
    }
    async fn get_token(
        &self,
        token: &AccessToken,
        token_id: &str,
    ) -> Result<ManagedToken, KSeFError> {
        self.tokens.get_token(token, token_id).await
    }
    async fn revoke_token(&self, token: &AccessToken, token_id: &str) -> Result<(), KSeFError> {
        self.tokens.revoke_token(token, token_id).await
    }
}

#[async_trait]
impl KSeFAuthSessions for KSeFApiClient {
    async fn list_sessions(&self, token: &AccessToken) -> Result<Vec<AuthSessionInfo>, KSeFError> {
        self.sessions.list_sessions(token).await
    }
    async fn revoke_session(&self, token: &AccessToken, reference: &str) -> Result<(), KSeFError> {
        self.sessions.revoke_session(token, reference).await
    }
    async fn revoke_current_session(&self, token: &AccessToken) -> Result<(), KSeFError> {
        self.sessions.revoke_current_session(token).await
    }
}

#[async_trait]
impl KSeFCertificates for KSeFApiClient {
    async fn get_limits(&self, token: &AccessToken) -> Result<CertificateLimits, KSeFError> {
        self.certificates.get_limits(token).await
    }
    async fn submit_enrollment(
        &self,
        token: &AccessToken,
        req: &CertificateEnrollmentRequest,
    ) -> Result<CertificateEnrollment, KSeFError> {
        self.certificates.submit_enrollment(token, req).await
    }
    async fn get_enrollment_status(
        &self,
        token: &AccessToken,
        reference: &str,
    ) -> Result<CertificateEnrollment, KSeFError> {
        self.certificates
            .get_enrollment_status(token, reference)
            .await
    }
    async fn query_certificates(
        &self,
        token: &AccessToken,
        req: &CertificateQueryRequest,
    ) -> Result<Vec<CertificateEnrollment>, KSeFError> {
        self.certificates.query_certificates(token, req).await
    }
    async fn revoke_certificate(
        &self,
        token: &AccessToken,
        reference: &str,
    ) -> Result<(), KSeFError> {
        self.certificates.revoke_certificate(token, reference).await
    }
}

#[async_trait]
impl KSeFPeppol for KSeFApiClient {
    async fn query_providers(
        &self,
        token: &AccessToken,
        req: &PeppolQueryRequest,
    ) -> Result<PeppolProvidersResponse, KSeFError> {
        self.peppol.query_providers(token, req).await
    }
}

#[async_trait]
impl KSeFRateLimits for KSeFApiClient {
    async fn get_effective_limits(
        &self,
        token: &AccessToken,
    ) -> Result<EffectiveApiRateLimits, KSeFError> {
        self.rate_limits.get_effective_limits(token).await
    }
    async fn get_context_limits(
        &self,
        token: &AccessToken,
    ) -> Result<Vec<ContextLimits>, KSeFError> {
        self.rate_limits.get_context_limits(token).await
    }
    async fn get_subject_limits(
        &self,
        token: &AccessToken,
    ) -> Result<Vec<SubjectLimits>, KSeFError> {
        self.rate_limits.get_subject_limits(token).await
    }
}
