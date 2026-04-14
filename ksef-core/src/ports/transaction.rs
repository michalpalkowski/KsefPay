use async_trait::async_trait;

use crate::error::RepositoryError;
use crate::ports::invoice_repository::InvoiceRepository;
use crate::ports::job_queue::JobQueue;

/// A transactional scope that implements repository and queue traits.
/// All operations within this scope are atomic — committed together or
/// rolled back together.
///
/// Call `.commit()` to persist. Dropping without commit rolls back.
#[async_trait]
pub trait AtomicScope: InvoiceRepository + JobQueue + Send {
    async fn commit(self: Box<Self>) -> Result<(), RepositoryError>;
}

/// Factory for creating atomic scopes (transactions).
#[async_trait]
pub trait AtomicScopeFactory: Send + Sync {
    async fn begin(&self) -> Result<Box<dyn AtomicScope>, RepositoryError>;
}
