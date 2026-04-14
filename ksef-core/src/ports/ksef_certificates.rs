use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::certificate_mgmt::{
    CertificateEnrollment, CertificateLimits, EnrollmentStatus, KsefCertificateType,
};
use crate::error::KSeFError;

#[derive(Debug, Clone)]
pub struct CertificateEnrollmentRequest {
    pub certificate_type: KsefCertificateType,
    pub csr_pem: String,
}

#[derive(Debug, Clone)]
pub struct CertificateQueryRequest {
    pub status: Option<EnrollmentStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Port: `KSeF` certificate management.
#[async_trait]
pub trait KSeFCertificates: Send + Sync {
    async fn get_limits(&self, access_token: &AccessToken) -> Result<CertificateLimits, KSeFError>;

    async fn submit_enrollment(
        &self,
        access_token: &AccessToken,
        request: &CertificateEnrollmentRequest,
    ) -> Result<CertificateEnrollment, KSeFError>;

    async fn get_enrollment_status(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<CertificateEnrollment, KSeFError>;

    async fn query_certificates(
        &self,
        access_token: &AccessToken,
        request: &CertificateQueryRequest,
    ) -> Result<Vec<CertificateEnrollment>, KSeFError>;

    async fn revoke_certificate(
        &self,
        access_token: &AccessToken,
        reference_number: &str,
    ) -> Result<(), KSeFError>;
}
