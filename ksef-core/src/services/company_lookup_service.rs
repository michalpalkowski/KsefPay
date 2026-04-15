use std::sync::Arc;

use crate::domain::company::CompanyInfo;
use crate::domain::nip::Nip;
use crate::ports::company_cache::CompanyCacheRepository;
use crate::ports::company_lookup::{CompanyLookup, CompanyLookupError};

const CACHE_TTL: chrono::Duration = chrono::Duration::hours(24);

/// Orchestrates company lookup: cache first, then external API.
pub struct CompanyLookupService {
    cache: Arc<dyn CompanyCacheRepository>,
    lookup: Arc<dyn CompanyLookup>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompanyLookupServiceError {
    #[error(transparent)]
    Lookup(#[from] CompanyLookupError),

    #[error("cache error: {0}")]
    Cache(#[from] crate::error::RepositoryError),
}

impl CompanyLookupService {
    #[must_use]
    pub fn new(
        cache: Arc<dyn CompanyCacheRepository>,
        lookup: Arc<dyn CompanyLookup>,
    ) -> Self {
        Self { cache, lookup }
    }

    /// Look up company by NIP. Returns cached data if fresh, otherwise fetches from API.
    pub async fn lookup(&self, nip: &Nip) -> Result<CompanyInfo, CompanyLookupServiceError> {
        if let Some(cached) = self.cache.get(nip).await?
            && cached.is_fresh(CACHE_TTL)
        {
            return Ok(cached);
        }

        // Cache miss or stale — fetch from API
        let info = self.lookup.lookup(nip).await?;
        self.cache.set(&info).await?;
        Ok(info)
    }
}
