use async_trait::async_trait;

use crate::domain::company::CompanyInfo;
use crate::domain::nip::Nip;

/// Why a company lookup failed.
#[derive(Debug, thiserror::Error)]
pub enum CompanyLookupError {
    /// NIP is valid but not found in any registry.
    #[error("NIP {0} not found in registry")]
    NotFound(Nip),

    /// External API returned an error.
    #[error("registry API error: {0}")]
    ApiError(String),
}

/// Port: fetch company data by NIP from an external registry.
///
/// Implementations: `WhiteListClient` (Biała Lista VAT).
#[async_trait]
pub trait CompanyLookup: Send + Sync {
    async fn lookup(&self, nip: &Nip) -> Result<CompanyInfo, CompanyLookupError>;
}
