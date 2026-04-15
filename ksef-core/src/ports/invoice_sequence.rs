use async_trait::async_trait;

use crate::domain::nip::Nip;
use crate::error::RepositoryError;

/// Port: atomic invoice number generation per seller NIP per month.
#[async_trait]
pub trait InvoiceSequenceRepository: Send + Sync {
    /// Atomically increment and return the next invoice number for the given
    /// seller NIP, year, and month. First call for a new (nip, year, month)
    /// returns 1.
    async fn next_number(
        &self,
        seller_nip: &Nip,
        year: i32,
        month: u32,
    ) -> Result<u32, RepositoryError>;
}
