mod queries;

// Keep old files for backward compat during migration, will remove after
pub mod invoice_repo;
pub mod job_queue;
pub mod session_repo;

use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::Mutex;

use crate::domain::environment::KSeFEnvironment;
use crate::domain::invoice::{Invoice, InvoiceId, InvoiceStatus};
use crate::domain::job::{Job, JobId};
use crate::domain::nip::Nip;
use crate::domain::session::KSeFNumber;
use crate::error::{QueueError, RepositoryError};
use crate::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use crate::ports::job_queue::JobQueue;
use crate::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use crate::ports::transaction::{AtomicScope, AtomicScopeFactory};

/// Run all migrations against the given pool.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(include_str!("../../../migrations/001_initial_schema.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/002_fetched_status_and_raw_xml.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/003_invoice_type_and_corrections.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/004_nullable_payment_fields.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

// =========================================================================
// Db — production entry point, wraps PgPool
// =========================================================================

/// Database handle. Wraps a connection pool and implements all repository
/// traits. Use `.tx()` to start an atomic transaction scope.
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Start a new transaction. Returns a `Tx` that implements the same
    /// repository traits as `Db` but executes all queries within a single
    /// database transaction.
    ///
    /// Call `tx.commit()` to persist. Dropping without commit rolls back.
    pub async fn tx(&self) -> Result<Tx, RepositoryError> {
        let transaction = self.pool.begin().await.map_err(RepositoryError::Database)?;
        Ok(Tx {
            inner: Mutex::new(Some(transaction)),
        })
    }
}

// --- Db: InvoiceRepository ---

#[async_trait]
impl InvoiceRepository for Db {
    async fn save(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        queries::invoice::save(&self.pool, invoice).await
    }
    async fn find_by_id(&self, id: &InvoiceId) -> Result<Invoice, RepositoryError> {
        queries::invoice::find_by_id(&self.pool, id).await
    }
    async fn update_status(
        &self,
        id: &InvoiceId,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError> {
        queries::invoice::update_status(&self.pool, id, status).await
    }
    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        ksef_number: &str,
    ) -> Result<(), RepositoryError> {
        queries::invoice::set_ksef_number(&self.pool, id, ksef_number).await
    }
    async fn set_ksef_error(&self, id: &InvoiceId, error: &str) -> Result<(), RepositoryError> {
        queries::invoice::set_ksef_error(&self.pool, id, error).await
    }
    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError> {
        queries::invoice::find_by_ksef_number(&self.pool, ksef_number).await
    }
    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        queries::invoice::upsert_by_ksef_number(&self.pool, invoice).await
    }
    async fn list(&self, filter: &InvoiceFilter) -> Result<Vec<Invoice>, RepositoryError> {
        queries::invoice::list(&self.pool, filter).await
    }
}

// --- Db: JobQueue ---

#[async_trait]
impl JobQueue for Db {
    async fn enqueue(&self, job: Job) -> Result<JobId, QueueError> {
        queries::job::enqueue(&self.pool, &job).await
    }
    async fn dequeue(&self) -> Result<Option<Job>, QueueError> {
        queries::job::dequeue(&self.pool).await
    }
    async fn complete(&self, id: &JobId) -> Result<(), QueueError> {
        queries::job::complete(&self.pool, id).await
    }
    async fn fail(&self, id: &JobId, error: &str) -> Result<(), QueueError> {
        queries::job::fail(&self.pool, id, error).await
    }
    async fn dead_letter(&self, id: &JobId, error: &str) -> Result<(), QueueError> {
        queries::job::dead_letter(&self.pool, id, error).await
    }
    async fn list_pending(&self) -> Result<Vec<Job>, QueueError> {
        queries::job::list_pending(&self.pool).await
    }
    async fn list_dead_letter(&self) -> Result<Vec<Job>, QueueError> {
        queries::job::list_dead_letter(&self.pool).await
    }
}

// --- Db: SessionRepository ---

#[async_trait]
impl SessionRepository for Db {
    async fn save_token_pair(&self, token: &StoredTokenPair) -> Result<(), RepositoryError> {
        queries::session::save_token_pair(&self.pool, token).await
    }
    async fn find_active_token(
        &self,
        nip: &Nip,
        env: KSeFEnvironment,
    ) -> Result<Option<StoredTokenPair>, RepositoryError> {
        queries::session::find_active_token(&self.pool, nip, env).await
    }
    async fn save_session(&self, session: &StoredSession) -> Result<(), RepositoryError> {
        queries::session::save_session(&self.pool, session).await
    }
    async fn find_active_session(
        &self,
        nip: &Nip,
        env: KSeFEnvironment,
    ) -> Result<Option<StoredSession>, RepositoryError> {
        queries::session::find_active_session(&self.pool, nip, env).await
    }
    async fn terminate_session(&self, session_id: uuid::Uuid) -> Result<(), RepositoryError> {
        queries::session::terminate_session(&self.pool, session_id).await
    }
}

// =========================================================================
// Tx — transaction scope, same traits, automatic rollback on drop
// =========================================================================

/// Atomic transaction scope. Implements the same repository traits as `Db`
/// but all queries execute within a single database transaction.
///
/// - Call `.commit()` to persist changes.
/// - Dropping without `.commit()` automatically rolls back.
pub struct Tx {
    inner: Mutex<Option<Transaction<'static, Postgres>>>,
}

impl Tx {
    /// Commit the transaction. Consumes self.
    pub async fn commit(self) -> Result<(), RepositoryError> {
        let opt = self.inner.into_inner();
        let transaction = opt.expect("tx already committed");
        sqlx::Transaction::commit(transaction)
            .await
            .map_err(RepositoryError::Database)
    }

    /// Borrow the inner connection for a query. Panics if already committed.
    async fn conn(&self) -> tokio::sync::MutexGuard<'_, Option<Transaction<'static, Postgres>>> {
        let guard = self.inner.lock().await;
        assert!(guard.is_some(), "transaction already committed");
        guard
    }
}

// --- Tx: InvoiceRepository ---

#[async_trait]
impl InvoiceRepository for Tx {
    async fn save(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::save(&mut **tx, invoice).await
    }
    async fn find_by_id(&self, id: &InvoiceId) -> Result<Invoice, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::find_by_id(&mut **tx, id).await
    }
    async fn update_status(
        &self,
        id: &InvoiceId,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::update_status(&mut **tx, id, status).await
    }
    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        ksef_number: &str,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::set_ksef_number(&mut **tx, id, ksef_number).await
    }
    async fn set_ksef_error(&self, id: &InvoiceId, error: &str) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::set_ksef_error(&mut **tx, id, error).await
    }
    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::find_by_ksef_number(&mut **tx, ksef_number).await
    }
    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::upsert_by_ksef_number(&mut **tx, invoice).await
    }
    async fn list(&self, filter: &InvoiceFilter) -> Result<Vec<Invoice>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::list(&mut **tx, filter).await
    }
}

// --- Tx: JobQueue ---

#[async_trait]
impl JobQueue for Tx {
    async fn enqueue(&self, job: Job) -> Result<JobId, QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::enqueue(&mut **tx, &job).await
    }
    async fn dequeue(&self) -> Result<Option<Job>, QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::dequeue(&mut **tx).await
    }
    async fn complete(&self, id: &JobId) -> Result<(), QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::complete(&mut **tx, id).await
    }
    async fn fail(&self, id: &JobId, error: &str) -> Result<(), QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::fail(&mut **tx, id, error).await
    }
    async fn dead_letter(&self, id: &JobId, error: &str) -> Result<(), QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::dead_letter(&mut **tx, id, error).await
    }
    async fn list_pending(&self) -> Result<Vec<Job>, QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::list_pending(&mut **tx).await
    }
    async fn list_dead_letter(&self) -> Result<Vec<Job>, QueueError> {
        let mut guard = self.inner.lock().await;
        let tx = guard.as_mut().expect("tx already committed");
        queries::job::list_dead_letter(&mut **tx).await
    }
}

// --- Tx: SessionRepository ---

#[async_trait]
impl SessionRepository for Tx {
    async fn save_token_pair(&self, token: &StoredTokenPair) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::session::save_token_pair(&mut **tx, token).await
    }
    async fn find_active_token(
        &self,
        nip: &Nip,
        env: KSeFEnvironment,
    ) -> Result<Option<StoredTokenPair>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::session::find_active_token(&mut **tx, nip, env).await
    }
    async fn save_session(&self, session: &StoredSession) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::session::save_session(&mut **tx, session).await
    }
    async fn find_active_session(
        &self,
        nip: &Nip,
        env: KSeFEnvironment,
    ) -> Result<Option<StoredSession>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::session::find_active_session(&mut **tx, nip, env).await
    }
    async fn terminate_session(&self, session_id: uuid::Uuid) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::session::terminate_session(&mut **tx, session_id).await
    }
}

// =========================================================================
// AtomicScope / AtomicScopeFactory — transaction ports
// =========================================================================

#[async_trait]
impl AtomicScope for Tx {
    async fn commit(self: Box<Self>) -> Result<(), RepositoryError> {
        let opt = self.inner.into_inner();
        let transaction = opt.expect("tx already committed");
        sqlx::Transaction::commit(transaction)
            .await
            .map_err(RepositoryError::Database)
    }
}

#[async_trait]
impl AtomicScopeFactory for Db {
    async fn begin(&self) -> Result<Box<dyn AtomicScope>, RepositoryError> {
        let transaction = self.pool.begin().await.map_err(RepositoryError::Database)?;
        Ok(Box::new(Tx {
            inner: Mutex::new(Some(transaction)),
        }))
    }
}
