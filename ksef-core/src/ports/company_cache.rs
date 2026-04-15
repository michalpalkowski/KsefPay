use async_trait::async_trait;

use crate::domain::company::CompanyInfo;
use crate::domain::nip::Nip;
use crate::error::RepositoryError;

/// Port: cached company data persistence.
#[async_trait]
pub trait CompanyCacheRepository: Send + Sync {
    /// Get cached company info. Returns `None` if not cached.
    async fn get(&self, nip: &Nip) -> Result<Option<CompanyInfo>, RepositoryError>;

    /// Store or update company info in cache.
    async fn set(&self, info: &CompanyInfo) -> Result<(), RepositoryError>;
}
