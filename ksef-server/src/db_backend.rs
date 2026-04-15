use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use ksef_core::infra::{pg, sqlite};
use ksef_core::ports::audit_log::AuditLogRepository;
use ksef_core::ports::company_cache::CompanyCacheRepository;
use ksef_core::ports::invoice_repository::InvoiceRepository;
use ksef_core::ports::invoice_sequence::InvoiceSequenceRepository;
use ksef_core::ports::job_queue::JobQueue;
use ksef_core::ports::local_token_repository::LocalTokenRepository;
use ksef_core::ports::nip_account_repository::NipAccountRepository;
use ksef_core::ports::session_repository::SessionRepository;
use ksef_core::ports::transaction::AtomicScopeFactory;
use ksef_core::ports::user_repository::UserRepository;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackendKind {
    Postgres,
    Sqlite,
}

#[derive(Clone)]
pub struct DatabasePorts {
    pub kind: DatabaseBackendKind,
    pub invoice_repo: Arc<dyn InvoiceRepository>,
    pub job_queue: Arc<dyn JobQueue>,
    pub session_repo: Arc<dyn SessionRepository>,
    pub atomic_scope_factory: Arc<dyn AtomicScopeFactory>,
    pub user_repo: Arc<dyn UserRepository>,
    pub nip_account_repo: Arc<dyn NipAccountRepository>,
    pub company_cache: Arc<dyn CompanyCacheRepository>,
    pub invoice_sequence: Arc<dyn InvoiceSequenceRepository>,
    pub audit_log_repo: Arc<dyn AuditLogRepository>,
    pub local_token_repo: Arc<dyn LocalTokenRepository>,
}

pub fn detect_backend_kind(database_url: &str) -> anyhow::Result<DatabaseBackendKind> {
    if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        return Ok(DatabaseBackendKind::Postgres);
    }
    if database_url.starts_with("sqlite://") {
        return Ok(DatabaseBackendKind::Sqlite);
    }

    Err(anyhow::anyhow!(
        "unsupported DATABASE_URL scheme; expected one of: postgres://, postgresql://, sqlite://"
    ))
}

fn ensure_sqlite_parent_dir(database_url: &str) -> anyhow::Result<()> {
    let raw = database_url
        .strip_prefix("sqlite://")
        .ok_or_else(|| anyhow::anyhow!("sqlite URL must start with sqlite://"))?;

    if raw.is_empty() {
        return Err(anyhow::anyhow!(
            "sqlite DATABASE_URL must include a database path, e.g. sqlite://./.data/ksef.db"
        ));
    }

    if raw == ":memory:" || raw.starts_with("file:") {
        return Ok(());
    }

    let path_without_query = raw.split('?').next().unwrap_or(raw);
    if path_without_query.is_empty() {
        return Err(anyhow::anyhow!(
            "sqlite DATABASE_URL must include a database path before query params"
        ));
    }

    let path = Path::new(path_without_query);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for sqlite database at '{}'",
                parent.display()
            )
        })?;
    }

    Ok(())
}

pub async fn connect(database_url: &str) -> anyhow::Result<DatabasePorts> {
    match detect_backend_kind(database_url)? {
        DatabaseBackendKind::Postgres => connect_postgres(database_url).await,
        DatabaseBackendKind::Sqlite => connect_sqlite(database_url).await,
    }
}

async fn connect_postgres(database_url: &str) -> anyhow::Result<DatabasePorts> {
    tracing::info!(backend = "postgres", "connecting to database");
    let pool = sqlx::PgPool::connect(database_url)
        .await
        .with_context(|| "failed to connect to PostgreSQL")?;

    tracing::info!(backend = "postgres", "running migrations");
    pg::run_migrations(&pool)
        .await
        .with_context(|| "failed to run PostgreSQL migrations")?;

    let db = Arc::new(pg::Db::new(pool));

    let invoice_repo: Arc<dyn InvoiceRepository> = db.clone();
    let job_queue: Arc<dyn JobQueue> = db.clone();
    let session_repo: Arc<dyn SessionRepository> = db.clone();
    let user_repo: Arc<dyn UserRepository> = db.clone();
    let nip_account_repo: Arc<dyn NipAccountRepository> = db.clone();
    let company_cache: Arc<dyn CompanyCacheRepository> = db.clone();
    let invoice_sequence: Arc<dyn InvoiceSequenceRepository> = db.clone();
    let audit_log_repo: Arc<dyn AuditLogRepository> = db.clone();
    let local_token_repo: Arc<dyn LocalTokenRepository> = db.clone();
    let atomic_scope_factory: Arc<dyn AtomicScopeFactory> = db;

    Ok(DatabasePorts {
        kind: DatabaseBackendKind::Postgres,
        invoice_repo,
        job_queue,
        session_repo,
        atomic_scope_factory,
        user_repo,
        nip_account_repo,
        company_cache,
        invoice_sequence,
        audit_log_repo,
        local_token_repo,
    })
}

async fn connect_sqlite(database_url: &str) -> anyhow::Result<DatabasePorts> {
    ensure_sqlite_parent_dir(database_url)?;

    tracing::info!(backend = "sqlite", database_url, "connecting to database");

    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| "failed to parse sqlite DATABASE_URL")?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| "failed to connect to SQLite")?;

    tracing::info!(backend = "sqlite", "running migrations");
    sqlite::run_migrations(&pool)
        .await
        .with_context(|| "failed to run SQLite migrations")?;

    let db = Arc::new(sqlite::Db::new(pool));

    let invoice_repo: Arc<dyn InvoiceRepository> = db.clone();
    let job_queue: Arc<dyn JobQueue> = db.clone();
    let session_repo: Arc<dyn SessionRepository> = db.clone();
    let user_repo: Arc<dyn UserRepository> = db.clone();
    let nip_account_repo: Arc<dyn NipAccountRepository> = db.clone();
    let company_cache: Arc<dyn CompanyCacheRepository> = db.clone();
    let invoice_sequence: Arc<dyn InvoiceSequenceRepository> = db.clone();
    let audit_log_repo: Arc<dyn AuditLogRepository> = db.clone();
    let local_token_repo: Arc<dyn LocalTokenRepository> = db.clone();
    let atomic_scope_factory: Arc<dyn AtomicScopeFactory> = db;

    Ok(DatabasePorts {
        kind: DatabaseBackendKind::Sqlite,
        invoice_repo,
        job_queue,
        session_repo,
        atomic_scope_factory,
        user_repo,
        nip_account_repo,
        company_cache,
        invoice_sequence,
        audit_log_repo,
        local_token_repo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_backend_kind_accepts_postgres_schemes() {
        assert_eq!(
            detect_backend_kind("postgres://user:pass@localhost:5432/db").unwrap(),
            DatabaseBackendKind::Postgres
        );
        assert_eq!(
            detect_backend_kind("postgresql://user:pass@localhost:5432/db").unwrap(),
            DatabaseBackendKind::Postgres
        );
    }

    #[test]
    fn detect_backend_kind_accepts_sqlite_scheme() {
        assert_eq!(
            detect_backend_kind("sqlite://./.data/ksef.db").unwrap(),
            DatabaseBackendKind::Sqlite
        );
    }

    #[test]
    fn detect_backend_kind_rejects_unknown_scheme() {
        let err = detect_backend_kind("mysql://root:root@localhost:3306/ksef").unwrap_err();
        assert!(
            err.to_string().contains("unsupported DATABASE_URL scheme"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ensure_sqlite_parent_dir_rejects_empty_path() {
        let err = ensure_sqlite_parent_dir("sqlite://").unwrap_err();
        assert!(
            err.to_string().contains("must include a database path"),
            "unexpected error: {err}"
        );
    }
}
