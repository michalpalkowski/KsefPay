mod queries;

use async_trait::async_trait;
use sqlx::{Sqlite, SqlitePool, Transaction};
use tokio::sync::Mutex;

use crate::domain::company::CompanyInfo;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::invoice::{Invoice, InvoiceId, InvoiceStatus};
use crate::domain::job::{Job, JobId};
use crate::domain::nip::Nip;
use crate::domain::nip_account::{NipAccount, NipAccountId};
use crate::domain::session::KSeFNumber;
use crate::domain::user::{User, UserId};
use crate::error::{QueueError, RepositoryError};
use crate::ports::company_cache::CompanyCacheRepository;
use crate::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use crate::ports::invoice_sequence::InvoiceSequenceRepository;
use crate::ports::job_queue::JobQueue;
use crate::ports::nip_account_repository::NipAccountRepository;
use crate::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use crate::ports::transaction::{AtomicScope, AtomicScopeFactory};
use crate::ports::user_repository::UserRepository;

/// Run all SQLite migrations against the given pool.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/001_initial_schema.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/002_fetched_status_and_raw_xml.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/003_invoice_type_and_corrections.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/004_nullable_payment_fields.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/005_multi_tenant_auth.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/006_company_cache_and_invoice_sequences.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

/// SQLite database handle. Implements all repository/queue/session ports.
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn tx(&self) -> Result<Tx, RepositoryError> {
        let transaction = self.pool.begin().await.map_err(RepositoryError::Database)?;
        Ok(Tx {
            inner: Mutex::new(Some(transaction)),
        })
    }
}

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

pub struct Tx {
    inner: Mutex<Option<Transaction<'static, Sqlite>>>,
}

impl Tx {
    pub async fn commit(self) -> Result<(), RepositoryError> {
        let opt = self.inner.into_inner();
        let transaction = opt.expect("tx already committed");
        sqlx::Transaction::commit(transaction)
            .await
            .map_err(RepositoryError::Database)
    }

    async fn conn(&self) -> tokio::sync::MutexGuard<'_, Option<Transaction<'static, Sqlite>>> {
        let guard = self.inner.lock().await;
        assert!(guard.is_some(), "transaction already committed");
        guard
    }
}

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

#[async_trait]
impl UserRepository for Db {
    async fn create(&self, user: &User) -> Result<UserId, RepositoryError> {
        queries::user::create(&self.pool, user).await
    }

    async fn find_by_id(&self, id: &UserId) -> Result<User, RepositoryError> {
        queries::user::find_by_id(&self.pool, id).await
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<User>, RepositoryError> {
        queries::user::find_by_email(&self.pool, email).await
    }

    async fn update_password(&self, user: &User) -> Result<(), RepositoryError> {
        queries::user::update_password(&self.pool, user).await
    }
}

#[async_trait]
impl UserRepository for Tx {
    async fn create(&self, user: &User) -> Result<UserId, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::user::create(&mut **tx, user).await
    }

    async fn find_by_id(&self, id: &UserId) -> Result<User, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::user::find_by_id(&mut **tx, id).await
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<User>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::user::find_by_email(&mut **tx, email).await
    }

    async fn update_password(&self, user: &User) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::user::update_password(&mut **tx, user).await
    }
}

#[async_trait]
impl InvoiceSequenceRepository for Db {
    async fn next_number(
        &self,
        seller_nip: &Nip,
        year: i32,
        month: u32,
    ) -> Result<u32, RepositoryError> {
        queries::invoice_sequence::next_number(&self.pool, seller_nip, year, month).await
    }
}

#[async_trait]
impl CompanyCacheRepository for Db {
    async fn get(&self, nip: &Nip) -> Result<Option<CompanyInfo>, RepositoryError> {
        queries::company_cache::get(&self.pool, nip).await
    }
    async fn set(&self, info: &CompanyInfo) -> Result<(), RepositoryError> {
        queries::company_cache::set(&self.pool, info).await
    }
}

#[async_trait]
impl NipAccountRepository for Db {
    async fn create(&self, account: &NipAccount) -> Result<NipAccountId, RepositoryError> {
        queries::nip_account::create(&self.pool, account).await
    }

    async fn find_by_id(&self, id: &NipAccountId) -> Result<NipAccount, RepositoryError> {
        queries::nip_account::find_by_id(&self.pool, id).await
    }

    async fn find_by_nip(&self, nip: &Nip) -> Result<Option<NipAccount>, RepositoryError> {
        queries::nip_account::find_by_nip(&self.pool, nip).await
    }

    async fn update_credentials(&self, account: &NipAccount) -> Result<(), RepositoryError> {
        queries::nip_account::update_credentials(&self.pool, account).await
    }

    async fn grant_access(
        &self,
        user_id: &UserId,
        account_id: &NipAccountId,
    ) -> Result<(), RepositoryError> {
        queries::nip_account::grant_access(&self.pool, user_id, account_id).await
    }

    async fn revoke_access(
        &self,
        user_id: &UserId,
        account_id: &NipAccountId,
    ) -> Result<(), RepositoryError> {
        queries::nip_account::revoke_access(&self.pool, user_id, account_id).await
    }

    async fn list_by_user(&self, user_id: &UserId) -> Result<Vec<NipAccount>, RepositoryError> {
        queries::nip_account::list_by_user(&self.pool, user_id).await
    }

    async fn has_access(
        &self,
        user_id: &UserId,
        nip: &Nip,
    ) -> Result<Option<NipAccount>, RepositoryError> {
        queries::nip_account::has_access(&self.pool, user_id, nip).await
    }
}

#[async_trait]
impl NipAccountRepository for Tx {
    async fn create(&self, account: &NipAccount) -> Result<NipAccountId, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::create(&mut **tx, account).await
    }

    async fn find_by_id(&self, id: &NipAccountId) -> Result<NipAccount, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::find_by_id(&mut **tx, id).await
    }

    async fn find_by_nip(&self, nip: &Nip) -> Result<Option<NipAccount>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::find_by_nip(&mut **tx, nip).await
    }

    async fn update_credentials(&self, account: &NipAccount) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::update_credentials(&mut **tx, account).await
    }

    async fn grant_access(
        &self,
        user_id: &UserId,
        account_id: &NipAccountId,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::grant_access(&mut **tx, user_id, account_id).await
    }

    async fn revoke_access(
        &self,
        user_id: &UserId,
        account_id: &NipAccountId,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::revoke_access(&mut **tx, user_id, account_id).await
    }

    async fn list_by_user(&self, user_id: &UserId) -> Result<Vec<NipAccount>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::list_by_user(&mut **tx, user_id).await
    }

    async fn has_access(
        &self,
        user_id: &UserId,
        nip: &Nip,
    ) -> Result<Option<NipAccount>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::has_access(&mut **tx, user_id, nip).await
    }
}

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
