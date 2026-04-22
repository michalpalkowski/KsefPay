mod queries;

use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::domain::account_scope::AccountScope;
use crate::domain::application_access::{ApplicationAccessInvite, ApplicationAccessInviteId};
use crate::domain::audit::{AuditLogEntry, NewAuditLogEntry};
use crate::domain::company::CompanyInfo;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::invoice::{Invoice, InvoiceId, InvoiceStatus};
use crate::domain::job::{Job, JobId};
use crate::domain::nip::Nip;
use crate::domain::nip_account::{NipAccount, NipAccountId};
use crate::domain::session::KSeFNumber;
use crate::domain::token_mgmt::LocalToken;
use crate::domain::user::{User, UserId};
use crate::domain::workspace::{
    Workspace, WorkspaceId, WorkspaceInvite, WorkspaceInviteId, WorkspaceMembership,
    WorkspaceNipOwnership, WorkspaceRole, WorkspaceSummary,
};
use crate::error::{QueueError, RepositoryError};
use crate::infra::crypto::CertificateSecretBox;
use crate::ports::application_access_repository::ApplicationAccessRepository;
use crate::ports::audit_log::AuditLogRepository;
use crate::ports::company_cache::CompanyCacheRepository;
use crate::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use crate::ports::invoice_sequence::InvoiceSequenceRepository;
use crate::ports::job_queue::JobQueue;
use crate::ports::local_token_repository::LocalTokenRepository;
use crate::ports::nip_account_repository::NipAccountRepository;
use crate::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use crate::ports::transaction::{AtomicScope, AtomicScopeFactory};
use crate::ports::user_repository::UserRepository;
use crate::ports::workspace_repository::WorkspaceRepository;

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
    sqlx::raw_sql(include_str!(
        "../../../migrations/005_multi_tenant_auth.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/006_company_cache_and_invoice_sequences.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/007_security_hardening.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/008_nip_account_tokens.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/009_invoice_composite_ksef_key.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/010_nip_credential_managers.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!("../../../migrations/011_workspaces.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/012_drop_user_nip_access.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/013_application_access_invites.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!("../../../migrations/014_invite_integrity.sql"))
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
    certificate_secret_box: Arc<CertificateSecretBox>,
}

impl Db {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self::with_certificate_secret_box(pool, Arc::new(CertificateSecretBox::insecure_dev()))
    }

    #[must_use]
    pub fn with_certificate_secret_box(
        pool: PgPool,
        certificate_secret_box: Arc<CertificateSecretBox>,
    ) -> Self {
        Self {
            pool,
            certificate_secret_box,
        }
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
            certificate_secret_box: self.certificate_secret_box.clone(),
        })
    }
}

// --- Db: InvoiceRepository ---

#[async_trait]
impl InvoiceRepository for Db {
    async fn save(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        queries::invoice::save(&self.pool, invoice).await
    }
    async fn find_by_id(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
    ) -> Result<Invoice, RepositoryError> {
        queries::invoice::find_by_id(&self.pool, id, scope).await
    }
    async fn update_status(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError> {
        queries::invoice::update_status(&self.pool, id, scope, status).await
    }
    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        ksef_number: &str,
    ) -> Result<(), RepositoryError> {
        queries::invoice::set_ksef_number(&self.pool, id, scope, ksef_number).await
    }
    async fn set_ksef_error(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        error: &str,
    ) -> Result<(), RepositoryError> {
        queries::invoice::set_ksef_error(&self.pool, id, scope, error).await
    }
    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError> {
        queries::invoice::find_by_ksef_number(&self.pool, ksef_number).await
    }
    async fn find_by_ksef_number_and_account(
        &self,
        ksef_number: &KSeFNumber,
        scope: &AccountScope,
    ) -> Result<Option<Invoice>, RepositoryError> {
        queries::invoice::find_by_ksef_number_and_account(&self.pool, ksef_number, scope).await
    }
    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        queries::invoice::upsert_by_ksef_number(&self.pool, invoice).await
    }
    async fn list(
        &self,
        scope: &AccountScope,
        filter: &InvoiceFilter,
    ) -> Result<Vec<Invoice>, RepositoryError> {
        queries::invoice::list(&self.pool, scope, filter).await
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

// --- Db: UserRepository ---

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
impl ApplicationAccessRepository for Db {
    async fn create_invite(
        &self,
        invite: &ApplicationAccessInvite,
    ) -> Result<ApplicationAccessInviteId, RepositoryError> {
        queries::application_access::create_invite(&self.pool, invite).await
    }

    async fn list_pending_invites(&self) -> Result<Vec<ApplicationAccessInvite>, RepositoryError> {
        queries::application_access::list_pending_invites(&self.pool).await
    }

    async fn find_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApplicationAccessInvite>, RepositoryError> {
        queries::application_access::find_invite_by_token_hash(&self.pool, token_hash).await
    }

    async fn activate_application_access(
        &self,
        invite_id: &ApplicationAccessInviteId,
        user_id: &UserId,
        user_email: &str,
    ) -> Result<WorkspaceSummary, RepositoryError> {
        let tx = self.tx().await?;
        let summary = {
            let mut guard = tx.conn().await;
            let inner = guard.as_mut().unwrap();
            let summary =
                queries::workspace::ensure_default_workspace_in_tx(inner, user_id, user_email)
                    .await?;
            queries::application_access::accept_invite(&mut **inner, invite_id).await?;
            summary
        };
        tx.commit().await?;
        Ok(summary)
    }

    async fn accept_invite(
        &self,
        invite_id: &ApplicationAccessInviteId,
    ) -> Result<(), RepositoryError> {
        queries::application_access::accept_invite(&self.pool, invite_id).await
    }

    async fn revoke_invite(
        &self,
        invite_id: &ApplicationAccessInviteId,
    ) -> Result<(), RepositoryError> {
        queries::application_access::revoke_invite(&self.pool, invite_id).await
    }
}

#[async_trait]
impl WorkspaceRepository for Db {
    async fn create_workspace(
        &self,
        workspace: &Workspace,
        owner_id: &UserId,
    ) -> Result<WorkspaceId, RepositoryError> {
        let tx = self.tx().await?;
        let workspace_id = {
            let mut guard = tx.conn().await;
            let inner = guard.as_mut().unwrap();
            queries::workspace::create_workspace_in_tx(inner, workspace, owner_id).await?
        };
        tx.commit().await?;
        Ok(workspace_id)
    }

    async fn ensure_default_workspace(
        &self,
        user_id: &UserId,
        user_email: &str,
    ) -> Result<WorkspaceSummary, RepositoryError> {
        let tx = self.tx().await?;
        let summary = {
            let mut guard = tx.conn().await;
            let inner = guard.as_mut().unwrap();
            queries::workspace::ensure_default_workspace_in_tx(inner, user_id, user_email).await?
        };
        tx.commit().await?;
        Ok(summary)
    }

    async fn find_by_id(&self, workspace_id: &WorkspaceId) -> Result<Workspace, RepositoryError> {
        queries::workspace::find_by_id(&self.pool, workspace_id).await
    }

    async fn list_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<Vec<WorkspaceSummary>, RepositoryError> {
        queries::workspace::list_for_user(&self.pool, user_id).await
    }

    async fn find_membership(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
    ) -> Result<Option<WorkspaceMembership>, RepositoryError> {
        queries::workspace::find_membership(&self.pool, workspace_id, user_id).await
    }

    async fn add_member(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
        role: WorkspaceRole,
    ) -> Result<(), RepositoryError> {
        queries::workspace::add_member(&self.pool, workspace_id, user_id, role).await
    }

    async fn attach_nip(
        &self,
        workspace_id: &WorkspaceId,
        account_id: &NipAccountId,
        ownership: WorkspaceNipOwnership,
        attached_by: &UserId,
    ) -> Result<(), RepositoryError> {
        queries::workspace::attach_nip(&self.pool, workspace_id, account_id, ownership, attached_by)
            .await
    }

    async fn list_nip_accounts_for_user(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
    ) -> Result<Vec<NipAccount>, RepositoryError> {
        queries::workspace::list_nip_accounts_for_user(
            &self.pool,
            workspace_id,
            user_id,
            &self.certificate_secret_box,
        )
        .await
    }

    async fn find_user_account_in_workspace(
        &self,
        workspace_id: &WorkspaceId,
        user_id: &UserId,
        nip: &Nip,
    ) -> Result<Option<(NipAccount, AccountScope, WorkspaceMembership)>, RepositoryError> {
        queries::workspace::find_user_account_in_workspace(
            &self.pool,
            workspace_id,
            user_id,
            nip,
            &self.certificate_secret_box,
        )
        .await
    }

    async fn create_invite(
        &self,
        invite: &WorkspaceInvite,
    ) -> Result<WorkspaceInviteId, RepositoryError> {
        queries::workspace::create_invite(&self.pool, invite).await
    }

    async fn list_pending_invites(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceInvite>, RepositoryError> {
        queries::workspace::list_pending_invites(&self.pool, workspace_id).await
    }

    async fn find_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<WorkspaceInvite>, RepositoryError> {
        queries::workspace::find_invite_by_token_hash(&self.pool, token_hash).await
    }

    async fn activate_invite_membership(
        &self,
        invite: &WorkspaceInvite,
        user_id: &UserId,
    ) -> Result<(), RepositoryError> {
        let tx = self.tx().await?;
        {
            let mut guard = tx.conn().await;
            let inner = guard.as_mut().unwrap();
            queries::workspace::add_member(&mut **inner, &invite.workspace_id, user_id, invite.role)
                .await?;
            queries::workspace::accept_invite(&mut **inner, &invite.id).await?;
        }
        tx.commit().await
    }

    async fn accept_invite(&self, invite_id: &WorkspaceInviteId) -> Result<(), RepositoryError> {
        queries::workspace::accept_invite(&self.pool, invite_id).await
    }

    async fn revoke_invite(&self, invite_id: &WorkspaceInviteId) -> Result<(), RepositoryError> {
        queries::workspace::revoke_invite(&self.pool, invite_id).await
    }
}

// --- Db: NipAccountRepository ---

#[async_trait]
impl NipAccountRepository for Db {
    async fn create(&self, account: &NipAccount) -> Result<NipAccountId, RepositoryError> {
        queries::nip_account::create(&self.pool, account, &self.certificate_secret_box).await
    }
    async fn find_by_id(&self, id: &NipAccountId) -> Result<NipAccount, RepositoryError> {
        queries::nip_account::find_by_id(&self.pool, id, &self.certificate_secret_box).await
    }
    async fn find_by_nip(&self, nip: &Nip) -> Result<Option<NipAccount>, RepositoryError> {
        queries::nip_account::find_by_nip(&self.pool, nip, &self.certificate_secret_box).await
    }
    async fn update_credentials(&self, account: &NipAccount) -> Result<(), RepositoryError> {
        queries::nip_account::update_credentials(&self.pool, account, &self.certificate_secret_box)
            .await
    }
}

// --- Db: InvoiceSequenceRepository ---

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

// --- Db: CompanyCacheRepository ---

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
impl AuditLogRepository for Db {
    async fn log(&self, entry: &NewAuditLogEntry) -> Result<(), RepositoryError> {
        queries::audit::log(&self.pool, entry).await
    }

    async fn list_recent(&self, limit: u32) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        queries::audit::list_recent(&self.pool, limit).await
    }
}

#[async_trait]
impl LocalTokenRepository for Db {
    async fn save(&self, token: &LocalToken) -> Result<(), RepositoryError> {
        queries::local_token::save(&self.pool, token).await
    }

    async fn list_by_account(
        &self,
        scope: &AccountScope,
    ) -> Result<Vec<LocalToken>, RepositoryError> {
        queries::local_token::list_by_account(&self.pool, scope).await
    }

    async fn list_by_account_for_user(
        &self,
        scope: &AccountScope,
        user_id: &UserId,
    ) -> Result<Vec<LocalToken>, RepositoryError> {
        queries::local_token::list_by_account_for_user(&self.pool, scope, user_id).await
    }

    async fn mark_revoked(
        &self,
        ksef_token_id: &str,
        scope: &AccountScope,
    ) -> Result<(), RepositoryError> {
        queries::local_token::mark_revoked(&self.pool, ksef_token_id, scope).await
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
    certificate_secret_box: Arc<CertificateSecretBox>,
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
    async fn find_by_id(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
    ) -> Result<Invoice, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::find_by_id(&mut **tx, id, scope).await
    }
    async fn update_status(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::update_status(&mut **tx, id, scope, status).await
    }
    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        ksef_number: &str,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::set_ksef_number(&mut **tx, id, scope, ksef_number).await
    }
    async fn set_ksef_error(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        error: &str,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::set_ksef_error(&mut **tx, id, scope, error).await
    }
    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::find_by_ksef_number(&mut **tx, ksef_number).await
    }
    async fn find_by_ksef_number_and_account(
        &self,
        ksef_number: &KSeFNumber,
        scope: &AccountScope,
    ) -> Result<Option<Invoice>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::find_by_ksef_number_and_account(&mut **tx, ksef_number, scope).await
    }
    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::upsert_by_ksef_number(&mut **tx, invoice).await
    }
    async fn list(
        &self,
        scope: &AccountScope,
        filter: &InvoiceFilter,
    ) -> Result<Vec<Invoice>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::invoice::list(&mut **tx, scope, filter).await
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

#[async_trait]
impl AuditLogRepository for Tx {
    async fn log(&self, entry: &NewAuditLogEntry) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::audit::log(&mut **tx, entry).await
    }

    async fn list_recent(&self, limit: u32) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::audit::list_recent(&mut **tx, limit).await
    }
}

// --- Tx: UserRepository ---

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

// --- Tx: NipAccountRepository ---

#[async_trait]
impl NipAccountRepository for Tx {
    async fn create(&self, account: &NipAccount) -> Result<NipAccountId, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::create(&mut **tx, account, &self.certificate_secret_box).await
    }
    async fn find_by_id(&self, id: &NipAccountId) -> Result<NipAccount, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::find_by_id(&mut **tx, id, &self.certificate_secret_box).await
    }
    async fn find_by_nip(&self, nip: &Nip) -> Result<Option<NipAccount>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::find_by_nip(&mut **tx, nip, &self.certificate_secret_box).await
    }
    async fn update_credentials(&self, account: &NipAccount) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::nip_account::update_credentials(&mut **tx, account, &self.certificate_secret_box)
            .await
    }
}

#[async_trait]
impl LocalTokenRepository for Tx {
    async fn save(&self, token: &LocalToken) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::local_token::save(&mut **tx, token).await
    }

    async fn list_by_account(
        &self,
        scope: &AccountScope,
    ) -> Result<Vec<LocalToken>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::local_token::list_by_account(&mut **tx, scope).await
    }

    async fn list_by_account_for_user(
        &self,
        scope: &AccountScope,
        user_id: &UserId,
    ) -> Result<Vec<LocalToken>, RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::local_token::list_by_account_for_user(&mut **tx, scope, user_id).await
    }

    async fn mark_revoked(
        &self,
        ksef_token_id: &str,
        scope: &AccountScope,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.conn().await;
        let tx = guard.as_mut().unwrap();
        queries::local_token::mark_revoked(&mut **tx, ksef_token_id, scope).await
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
            certificate_secret_box: self.certificate_secret_box.clone(),
        }))
    }
}
