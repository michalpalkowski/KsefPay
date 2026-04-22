//! Integration tests for SQLite repository and queue implementations.
//!
//! Each test gets its own isolated database file — fully parallel-safe.

use std::str::FromStr;
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use uuid::Uuid;

use ksef_core::domain::account_scope::AccountScope;
use ksef_core::domain::auth::{AccessToken, RefreshToken, TokenPair};
use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
    LineItem, Money, Party, PaymentMethod, Quantity, VatRate,
};
use ksef_core::domain::job::{Job, JobId, JobStatus};
use ksef_core::domain::nip::Nip;
use ksef_core::domain::nip_account::{KSeFAuthMethod, NipAccount, NipAccountId};
use ksef_core::domain::session::{KSeFNumber, SessionReference};
use ksef_core::domain::user::{User, UserId};
use ksef_core::domain::workspace::{WorkspaceInvite, WorkspaceInviteId, WorkspaceRole};
use ksef_core::error::RepositoryError;
use ksef_core::infra::sqlite::{Db, run_migrations};
use ksef_core::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use ksef_core::ports::job_queue::JobQueue;
use ksef_core::ports::nip_account_repository::NipAccountRepository;
use ksef_core::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use ksef_core::ports::user_repository::UserRepository;
use ksef_core::ports::workspace_repository::WorkspaceRepository;
use ksef_core::test_support::fixtures::make_scope;

// ---------------------------------------------------------------------------
// Shared infrastructure
// ---------------------------------------------------------------------------

async fn isolated_pool() -> SqlitePool {
    let db_path = std::env::temp_dir().join(format!("ksef_sqlite_{}.db", Uuid::new_v4()));
    let database_url = format!("sqlite://{}", db_path.display());

    let options = SqliteConnectOptions::from_str(&database_url)
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();

    run_migrations(&pool).await.unwrap();
    pool
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn sample_invoice() -> Invoice {
    let seller_nip = Nip::parse("5260250274").unwrap();
    let buyer_nip = Nip::parse("5260250274").unwrap();

    Invoice {
        id: InvoiceId::new(),
        nip_account_id: test_account_id(),
        direction: Direction::Outgoing,
        status: InvoiceStatus::Draft,
        invoice_type: InvoiceType::Vat,
        invoice_number: "FV/2026/04/001".to_string(),
        issue_date: NaiveDate::from_ymd_opt(2026, 4, 13).unwrap(),
        sale_date: Some(NaiveDate::from_ymd_opt(2026, 4, 13).unwrap()),
        corrected_invoice_number: None,
        correction_reason: None,
        original_ksef_number: None,
        advance_payment_date: None,
        seller: Party {
            nip: Some(seller_nip),
            name: "Test Seller Sp. z o.o.".to_string(),
            address: Address {
                country_code: CountryCode::pl(),
                line1: "ul. Testowa 1".to_string(),
                line2: "00-001 Warszawa".to_string(),
            },
        },
        buyer: Party {
            nip: Some(buyer_nip),
            name: "Test Buyer S.A.".to_string(),
            address: Address {
                country_code: CountryCode::pl(),
                line1: "ul. Kupiecka 5".to_string(),
                line2: "00-002 Krakow".to_string(),
            },
        },
        currency: Currency::pln(),
        line_items: vec![LineItem {
            line_number: 1,
            description: "Uslugi programistyczne".to_string(),
            unit: Some("godz".to_string()),
            quantity: Quantity::integer(160),
            unit_net_price: Some(Money::from_pln(150, 0)),
            net_value: Money::from_pln(24000, 0),
            vat_rate: VatRate::Rate23,
            vat_amount: Money::from_pln(5520, 0),
            gross_value: Money::from_pln(29520, 0),
        }],
        total_net: Money::from_pln(24000, 0),
        total_vat: Money::from_pln(5520, 0),
        total_gross: Money::from_pln(29520, 0),
        payment_method: Some(PaymentMethod::Transfer),
        payment_deadline: Some(NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()),
        bank_account: Some("PL12345678901234567890123456".to_string()),
        ksef_number: None,
        ksef_error: None,
        raw_xml: None,
    }
}

fn test_account_id() -> NipAccountId {
    NipAccountId::from_uuid(Uuid::from_u128(1))
}

fn account_id_a() -> NipAccountId {
    NipAccountId::from_uuid(Uuid::from_u128(11))
}

fn account_id_b() -> NipAccountId {
    NipAccountId::from_uuid(Uuid::from_u128(12))
}

fn account_id_c() -> NipAccountId {
    NipAccountId::from_uuid(Uuid::from_u128(13))
}

fn test_scope() -> AccountScope {
    make_scope(test_account_id(), test_nip())
}

fn scope_a() -> AccountScope {
    make_scope(account_id_a(), Nip::parse("5260250274").unwrap())
}

fn scope_b() -> AccountScope {
    make_scope(account_id_b(), Nip::parse("1060000062").unwrap())
}

fn scope_c() -> AccountScope {
    make_scope(account_id_c(), Nip::parse("7740001454").unwrap())
}

async fn create_account_with_id(repo: &Db, id: NipAccountId, nip: Nip) {
    let mut account = make_nip_account(&nip);
    account.id = id;
    NipAccountRepository::create(repo, &account).await.unwrap();
}

fn make_job(job_type: &str) -> Job {
    Job {
        id: JobId::new(),
        job_type: job_type.to_string(),
        payload: serde_json::json!({"invoice_id": "test-123"}),
        status: JobStatus::Pending,
        attempts: 0,
        max_attempts: 3,
        last_error: None,
        created_at: Utc::now(),
    }
}

fn test_nip() -> Nip {
    Nip::parse("5260250274").unwrap()
}

fn other_nip() -> Nip {
    Nip::parse("1060000062").unwrap()
}

fn make_token_pair(access_mins: i64, refresh_days: i64) -> TokenPair {
    TokenPair {
        access_token: AccessToken::new("access-token-value".to_string()),
        refresh_token: RefreshToken::new("refresh-token-value".to_string()),
        access_token_expires_at: Utc::now() + chrono::Duration::minutes(access_mins),
        refresh_token_expires_at: Utc::now() + chrono::Duration::days(refresh_days),
    }
}

fn make_stored_token(env: KSeFEnvironment) -> StoredTokenPair {
    StoredTokenPair {
        id: Uuid::new_v4(),
        nip: test_nip(),
        environment: env,
        token_pair: make_token_pair(15, 7),
        created_at: Utc::now(),
    }
}

fn make_stored_session(env: KSeFEnvironment) -> StoredSession {
    StoredSession {
        id: Uuid::new_v4(),
        session_reference: SessionReference::new(format!("session-ref-{}", Uuid::new_v4())),
        nip: test_nip(),
        environment: env,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(12),
        terminated_at: None,
    }
}

fn make_user(email: &str) -> User {
    User {
        id: UserId::new(),
        email: email.to_string(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$fake_salt$fake_hash".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_nip_account(nip: &Nip) -> NipAccount {
    NipAccount {
        id: NipAccountId::new(),
        nip: nip.clone(),
        display_name: format!("Firma {}", nip.as_str()),
        ksef_auth_method: KSeFAuthMethod::Xades,
        ksef_auth_token: None,
        cert_pem: None,
        key_pem: None,
        cert_auto_generated: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

// ===========================================================================
// Migration idempotency
// ===========================================================================

#[tokio::test]
async fn migrations_are_idempotent() {
    let pool = isolated_pool().await;
    // Run migrations a second time on the same DB — must not fail.
    run_migrations(&pool).await.unwrap();
}

#[tokio::test]
async fn migrations_run_three_times_without_error() {
    let pool = isolated_pool().await;
    run_migrations(&pool).await.unwrap();
    run_migrations(&pool).await.unwrap();
}

// ===========================================================================
// Invoice Repository
// ===========================================================================

#[tokio::test]
async fn invoice_save_and_find_by_id() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id, &test_scope())
        .await
        .unwrap();
    assert_eq!(found.id.as_uuid(), invoice.id.as_uuid());
    assert_eq!(found.invoice_number, invoice.invoice_number);
    assert_eq!(found.payment_method, Some(PaymentMethod::Transfer));
}

#[tokio::test]
async fn invoice_find_by_id_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;
    let err = InvoiceRepository::find_by_id(&repo, &InvoiceId::new(), &test_scope())
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn invoice_save_duplicate_returns_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;
    let invoice = sample_invoice();
    repo.save(&invoice).await.unwrap();
    let err = repo.save(&invoice).await.unwrap_err();
    assert!(matches!(err, RepositoryError::Duplicate { .. }));
}

#[tokio::test]
async fn invoice_update_status() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;
    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();
    repo.update_status(&id, &test_scope(), InvoiceStatus::Queued)
        .await
        .unwrap();
    let found = InvoiceRepository::find_by_id(&repo, &id, &test_scope())
        .await
        .unwrap();
    assert_eq!(found.status, InvoiceStatus::Queued);
}

#[tokio::test]
async fn invoice_set_ksef_number() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;
    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();
    repo.set_ksef_number(&id, &test_scope(), "KSeF-12345")
        .await
        .unwrap();
    let found = InvoiceRepository::find_by_id(&repo, &id, &test_scope())
        .await
        .unwrap();
    assert_eq!(found.ksef_number.unwrap().as_str(), "KSeF-12345");
}

#[tokio::test]
async fn invoice_set_ksef_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;
    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();
    repo.set_ksef_error(&id, &test_scope(), "timeout")
        .await
        .unwrap();
    let found = InvoiceRepository::find_by_id(&repo, &id, &test_scope())
        .await
        .unwrap();
    assert_eq!(found.ksef_error.as_deref(), Some("timeout"));
}

#[tokio::test]
async fn invoice_upsert_by_ksef_number_updates_existing() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let mut invoice = sample_invoice();
    invoice.ksef_number = Some(KSeFNumber::new("KSeF-SQLITE-001".to_string()));
    let first_id = repo.upsert_by_ksef_number(&invoice).await.unwrap();

    invoice.invoice_number = "FV/2026/04/002".to_string();
    let second_id = repo.upsert_by_ksef_number(&invoice).await.unwrap();

    assert_eq!(first_id.as_uuid(), second_id.as_uuid());
    let rows = repo
        .list(
            &test_scope(),
            &InvoiceFilter::new().with_direction(Direction::Outgoing),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].invoice_number, "FV/2026/04/002");
}

#[tokio::test]
async fn invoice_list_filters_by_direction() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let mut outgoing = sample_invoice();
    outgoing.direction = Direction::Outgoing;
    repo.save(&outgoing).await.unwrap();

    let mut incoming = sample_invoice();
    incoming.direction = Direction::Incoming;
    repo.save(&incoming).await.unwrap();

    let result = repo
        .list(
            &test_scope(),
            &InvoiceFilter::new().with_direction(Direction::Outgoing),
        )
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Outgoing);
}

#[tokio::test]
async fn invoice_list_filters_by_status() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let inv = sample_invoice();
    let id = repo.save(&inv).await.unwrap();
    repo.update_status(&id, &test_scope(), InvoiceStatus::Queued)
        .await
        .unwrap();

    let inv2 = sample_invoice();
    repo.save(&inv2).await.unwrap(); // stays Draft

    let result = repo
        .list(
            &test_scope(),
            &InvoiceFilter::new().with_status(InvoiceStatus::Queued),
        )
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
}

// ===========================================================================
// Invoice Filter: account_id tenant isolation
// ===========================================================================

#[tokio::test]
async fn invoice_account_id_filters_by_owner() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    let nip_c = Nip::parse("7740001454").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a.clone()).await;
    create_account_with_id(&repo, account_id_b(), nip_b.clone()).await;
    create_account_with_id(&repo, account_id_c(), nip_c.clone()).await;

    // Invoice 1 belongs to account A
    let mut inv1 = sample_invoice();
    inv1.nip_account_id = account_id_a();
    repo.save(&inv1).await.unwrap();

    // Invoice 2 belongs to account C
    let mut inv2 = sample_invoice();
    inv2.nip_account_id = account_id_c();
    repo.save(&inv2).await.unwrap();

    // Invoice 3 belongs to account B
    let mut inv3 = sample_invoice();
    inv3.nip_account_id = account_id_b();
    repo.save(&inv3).await.unwrap();

    // Filter by account_id=A → only inv1
    let result = repo.list(&scope_a(), &InvoiceFilter::new()).await.unwrap();
    assert_eq!(result.len(), 1);

    // Filter by account_id=C → only inv2
    let result = repo.list(&scope_c(), &InvoiceFilter::new()).await.unwrap();
    assert_eq!(result.len(), 1);

    // Filter by account_id=B → only inv3
    let result = repo.list(&scope_b(), &InvoiceFilter::new()).await.unwrap();
    assert_eq!(result.len(), 1);
}

#[tokio::test]
async fn invoice_account_id_combined_with_direction() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a.clone()).await;
    create_account_with_id(&repo, account_id_b(), nip_b.clone()).await;

    // Outgoing invoice in account A
    let mut inv1 = sample_invoice();
    inv1.direction = Direction::Outgoing;
    inv1.nip_account_id = account_id_a();
    repo.save(&inv1).await.unwrap();

    // Incoming invoice in account A
    let mut inv2 = sample_invoice();
    inv2.direction = Direction::Incoming;
    inv2.nip_account_id = account_id_a();
    repo.save(&inv2).await.unwrap();

    // account_id=A + direction=Outgoing → only inv1
    let result = repo
        .list(
            &scope_a(),
            &InvoiceFilter::new().with_direction(Direction::Outgoing),
        )
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Outgoing);

    // account_id=A + direction=Incoming → only inv2
    let result = repo
        .list(
            &scope_a(),
            &InvoiceFilter::new().with_direction(Direction::Incoming),
        )
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Incoming);
}

#[tokio::test]
async fn invoice_account_id_returns_empty_for_unrelated_account() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    let nip_unrelated = Nip::parse("7740001454").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a.clone()).await;
    create_account_with_id(&repo, account_id_b(), nip_b.clone()).await;
    create_account_with_id(&repo, account_id_c(), nip_unrelated).await;

    let mut inv = sample_invoice();
    inv.nip_account_id = account_id_a();
    repo.save(&inv).await.unwrap();

    let result = repo.list(&scope_c(), &InvoiceFilter::new()).await.unwrap();
    assert!(result.is_empty());
}

// ===========================================================================
// Job Queue
// ===========================================================================

#[tokio::test]
async fn job_enqueue_and_dequeue() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);
    let id = queue.enqueue(make_job("test")).await.unwrap();
    let dequeued = queue.dequeue().await.unwrap().unwrap();
    assert_eq!(dequeued.id.as_uuid(), id.as_uuid());
    assert_eq!(dequeued.status, JobStatus::Running);
}

#[tokio::test]
async fn job_dequeue_empty_returns_none() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);
    assert!(queue.dequeue().await.unwrap().is_none());
}

#[tokio::test]
async fn job_complete() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);
    let id = queue.enqueue(make_job("test")).await.unwrap();
    queue.dequeue().await.unwrap();
    queue.complete(&id).await.unwrap();
    assert!(queue.list_pending().await.unwrap().is_empty());
}

#[tokio::test]
async fn job_dequeue_concurrent_returns_distinct_jobs() {
    let pool = isolated_pool().await;

    let queue = Db::new(pool.clone());
    let id_a = queue.enqueue(make_job("concurrent_a")).await.unwrap();
    let id_b = queue.enqueue(make_job("concurrent_b")).await.unwrap();

    let queue_1 = Db::new(pool.clone());
    let queue_2 = Db::new(pool.clone());

    let (result_1, result_2) = tokio::join!(queue_1.dequeue(), queue_2.dequeue());

    let dequeued_1 = result_1.unwrap().expect("first dequeue should get a job");
    let dequeued_2 = result_2.unwrap().expect("second dequeue should get a job");

    assert_ne!(dequeued_1.id.as_uuid(), dequeued_2.id.as_uuid());

    let mut ids = vec![dequeued_1.id.as_uuid(), dequeued_2.id.as_uuid()];
    ids.sort();
    let mut expected = vec![id_a.as_uuid(), id_b.as_uuid()];
    expected.sort();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn job_fail_requeues_and_updates_attempts() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool.clone());

    let id = queue.enqueue(make_job("submit_invoice")).await.unwrap();
    queue.dequeue().await.unwrap();

    queue.fail(&id, "temporary failure").await.unwrap();

    let row = sqlx::query_as::<_, (i32, Option<String>, String)>(
        "SELECT attempts, last_error, status FROM jobs WHERE id = ?1",
    )
    .bind(id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, 1);
    assert_eq!(row.1.as_deref(), Some("temporary failure"));
    assert_eq!(row.2, "pending");
}

#[tokio::test]
async fn job_dead_letter_after_max_attempts() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let mut job = make_job("test");
    job.max_attempts = 2;
    let id = queue.enqueue(job).await.unwrap();
    queue.dequeue().await.unwrap();
    queue.fail(&id, "err 1").await.unwrap();
    queue.fail(&id, "err 2").await.unwrap();

    let dead = queue.list_dead_letter().await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].last_error.as_deref(), Some("err 2"));
}

// ===========================================================================
// Session Repository
// ===========================================================================

#[tokio::test]
async fn session_save_and_find_active_token() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let token = make_stored_token(KSeFEnvironment::Test);
    repo.save_token_pair(&token).await.unwrap();

    let found = repo
        .find_active_token(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap();

    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.nip.as_str(), "5260250274");
    assert_eq!(found.environment, KSeFEnvironment::Test);
}

#[tokio::test]
async fn session_wrong_env_returns_none() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    repo.save_token_pair(&make_stored_token(KSeFEnvironment::Test))
        .await
        .unwrap();
    let found = repo
        .find_active_token(&test_nip(), KSeFEnvironment::Production)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn session_save_find_and_terminate() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let session = make_stored_session(KSeFEnvironment::Test);
    let session_id = session.id;
    repo.save_session(&session).await.unwrap();

    let active = repo
        .find_active_session(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap();
    assert!(active.is_some());

    repo.terminate_session(session_id).await.unwrap();

    let active_after = repo
        .find_active_session(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap();
    assert!(active_after.is_none());
}

#[tokio::test]
async fn session_terminate_missing_returns_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let err = repo.terminate_session(Uuid::new_v4()).await.unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

// ===========================================================================
// User Repository
// ===========================================================================

#[tokio::test]
async fn user_create_and_find_by_id() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let user = make_user("alice@example.com");
    let id = UserRepository::create(&repo, &user).await.unwrap();

    let found = UserRepository::find_by_id(&repo, &id).await.unwrap();
    assert_eq!(found.email, "alice@example.com");
    assert_eq!(found.id.as_uuid(), user.id.as_uuid());
}

#[tokio::test]
async fn user_find_by_email() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let user = make_user("bob@example.com");
    UserRepository::create(&repo, &user).await.unwrap();

    let found = UserRepository::find_by_email(&repo, "bob@example.com")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().email, "bob@example.com");
}

#[tokio::test]
async fn user_find_by_email_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let found = UserRepository::find_by_email(&repo, "nobody@example.com")
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn user_find_by_id_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let err = UserRepository::find_by_id(&repo, &UserId::new())
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn user_duplicate_email_returns_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let user1 = make_user("dup@example.com");
    UserRepository::create(&repo, &user1).await.unwrap();

    let user2 = make_user("dup@example.com");
    let err = UserRepository::create(&repo, &user2).await.unwrap_err();
    assert!(matches!(err, RepositoryError::Duplicate { .. }));
}

#[tokio::test]
async fn user_different_emails_both_succeed() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    UserRepository::create(&repo, &make_user("a@example.com"))
        .await
        .unwrap();
    UserRepository::create(&repo, &make_user("b@example.com"))
        .await
        .unwrap();
}

// ===========================================================================
// NIP Account Repository
// ===========================================================================

#[tokio::test]
async fn nip_account_create_and_find_by_id() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let account = make_nip_account(&test_nip());
    let id = NipAccountRepository::create(&repo, &account).await.unwrap();

    let found = NipAccountRepository::find_by_id(&repo, &id).await.unwrap();
    assert_eq!(found.nip.as_str(), "5260250274");
    assert_eq!(found.display_name, account.display_name);
    assert_eq!(found.ksef_auth_method, KSeFAuthMethod::Xades);
}

#[tokio::test]
async fn nip_account_find_by_nip() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let account = make_nip_account(&test_nip());
    NipAccountRepository::create(&repo, &account).await.unwrap();

    let found = NipAccountRepository::find_by_nip(&repo, &test_nip())
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().nip.as_str(), "5260250274");
}

#[tokio::test]
async fn nip_account_find_by_nip_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let found = NipAccountRepository::find_by_nip(&repo, &other_nip())
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn nip_account_duplicate_nip_returns_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let acc1 = make_nip_account(&test_nip());
    NipAccountRepository::create(&repo, &acc1).await.unwrap();

    let acc2 = make_nip_account(&test_nip());
    let err = NipAccountRepository::create(&repo, &acc2)
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Duplicate { .. }));
}

#[tokio::test]
async fn nip_account_update_credentials() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let mut account = make_nip_account(&test_nip());
    NipAccountRepository::create(&repo, &account).await.unwrap();

    account.ksef_auth_method = KSeFAuthMethod::Token;
    account.ksef_auth_token = Some("my-token".to_string());
    account.cert_pem =
        Some(b"-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----".to_vec());
    account.key_pem =
        Some(b"-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----".to_vec());
    account.cert_auto_generated = true;
    NipAccountRepository::update_credentials(&repo, &account)
        .await
        .unwrap();

    let found = NipAccountRepository::find_by_nip(&repo, &test_nip())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.ksef_auth_method, KSeFAuthMethod::Token);
    assert_eq!(found.ksef_auth_token.as_deref(), Some("my-token"));
    assert!(found.cert_pem.is_some());
    assert!(found.key_pem.is_some());
    assert!(found.cert_auto_generated);
}

// ===========================================================================
// Workspace-scoped NIP access
// ===========================================================================

async fn ensure_default_workspace(
    db: &Db,
    user: &User,
) -> ksef_core::domain::workspace::WorkspaceSummary {
    WorkspaceRepository::ensure_default_workspace(db, &user.id, &user.email)
        .await
        .unwrap()
}

async fn attach_workspace_account(
    db: &Db,
    workspace_id: &ksef_core::domain::workspace::WorkspaceId,
    attached_by: &UserId,
    account_id: &NipAccountId,
) {
    WorkspaceRepository::attach_nip(
        db,
        workspace_id,
        account_id,
        ksef_core::domain::workspace::WorkspaceNipOwnership::WorkspaceOwned,
        attached_by,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn workspace_lists_accounts_for_active_member() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("owner@firm.pl");
    UserRepository::create(&db, &user).await.unwrap();
    let workspace = ensure_default_workspace(&db, &user).await;

    let acc1 = make_nip_account(&test_nip());
    let acc1_id = NipAccountRepository::create(&db, &acc1).await.unwrap();
    let acc2 = make_nip_account(&other_nip());
    let acc2_id = NipAccountRepository::create(&db, &acc2).await.unwrap();

    attach_workspace_account(&db, &workspace.workspace.id, &user.id, &acc1_id).await;
    attach_workspace_account(&db, &workspace.workspace.id, &user.id, &acc2_id).await;

    let accounts =
        WorkspaceRepository::list_nip_accounts_for_user(&db, &workspace.workspace.id, &user.id)
            .await
            .unwrap();
    assert_eq!(accounts.len(), 2);
}

#[tokio::test]
async fn workspace_lookup_returns_account_for_member() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("u@x.pl");
    UserRepository::create(&db, &user).await.unwrap();
    let workspace = ensure_default_workspace(&db, &user).await;

    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();
    attach_workspace_account(&db, &workspace.workspace.id, &user.id, &acc_id).await;

    let result = WorkspaceRepository::find_user_account_in_workspace(
        &db,
        &workspace.workspace.id,
        &user.id,
        &test_nip(),
    )
    .await
    .unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn workspace_lookup_returns_none_when_nip_not_attached() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("u@x.pl");
    UserRepository::create(&db, &user).await.unwrap();
    let workspace = ensure_default_workspace(&db, &user).await;

    NipAccountRepository::create(&db, &make_nip_account(&test_nip()))
        .await
        .unwrap();

    let result = WorkspaceRepository::find_user_account_in_workspace(
        &db,
        &workspace.workspace.id,
        &user.id,
        &test_nip(),
    )
    .await
    .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn workspace_isolation_between_users_is_enforced() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let alice = make_user("alice@x.pl");
    UserRepository::create(&db, &alice).await.unwrap();
    let bob = make_user("bob@x.pl");
    UserRepository::create(&db, &bob).await.unwrap();

    let alice_workspace = ensure_default_workspace(&db, &alice).await;
    let bob_workspace = ensure_default_workspace(&db, &bob).await;

    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();
    attach_workspace_account(&db, &alice_workspace.workspace.id, &alice.id, &acc_id).await;

    assert!(
        WorkspaceRepository::find_user_account_in_workspace(
            &db,
            &alice_workspace.workspace.id,
            &alice.id,
            &test_nip(),
        )
        .await
        .unwrap()
        .is_some()
    );
    assert!(
        WorkspaceRepository::find_user_account_in_workspace(
            &db,
            &bob_workspace.workspace.id,
            &bob.id,
            &test_nip(),
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn workspace_role_controls_credential_management() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let owner = make_user("owner@x.pl");
    UserRepository::create(&db, &owner).await.unwrap();
    let operator = make_user("operator@x.pl");
    UserRepository::create(&db, &operator).await.unwrap();

    let workspace = ensure_default_workspace(&db, &owner).await;
    WorkspaceRepository::add_member(
        &db,
        &workspace.workspace.id,
        &operator.id,
        WorkspaceRole::Operator,
    )
    .await
    .unwrap();

    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();
    attach_workspace_account(&db, &workspace.workspace.id, &owner.id, &acc_id).await;

    let (_, _, owner_membership) = WorkspaceRepository::find_user_account_in_workspace(
        &db,
        &workspace.workspace.id,
        &owner.id,
        &test_nip(),
    )
    .await
    .unwrap()
    .unwrap();
    let (_, _, operator_membership) = WorkspaceRepository::find_user_account_in_workspace(
        &db,
        &workspace.workspace.id,
        &operator.id,
        &test_nip(),
    )
    .await
    .unwrap()
    .unwrap();

    assert!(owner_membership.can_manage_credentials);
    assert!(!operator_membership.can_manage_credentials);
}

// ===========================================================================
// Cross-tenant isolation: mutations must not touch a different account's data
// ===========================================================================

/// `update_status` with a wrong `account_id` must return NotFound and must not
/// modify the invoice that belongs to the correct account.
#[tokio::test]
async fn update_status_wrong_account_returns_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a).await;
    create_account_with_id(&repo, account_id_b(), nip_b).await;

    let mut inv = sample_invoice();
    inv.nip_account_id = account_id_a();
    let id = repo.save(&inv).await.unwrap();

    // Try to update with account B's scope — must fail
    let err = repo
        .update_status(&id, &scope_b(), InvoiceStatus::Queued)
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));

    // Invoice A is unchanged
    let found = InvoiceRepository::find_by_id(&repo, &id, &scope_a())
        .await
        .unwrap();
    assert_eq!(found.status, InvoiceStatus::Draft);
}

/// `set_ksef_number` with a wrong `account_id` must return NotFound and must not
/// write the ksef_number on the invoice belonging to the correct account.
#[tokio::test]
async fn set_ksef_number_wrong_account_returns_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a).await;
    create_account_with_id(&repo, account_id_b(), nip_b).await;

    let mut inv = sample_invoice();
    inv.nip_account_id = account_id_a();
    let id = repo.save(&inv).await.unwrap();

    let err = repo
        .set_ksef_number(&id, &scope_b(), "KSeF-EVIL-001")
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));

    // ksef_number on account A's invoice is still None
    let found = InvoiceRepository::find_by_id(&repo, &id, &scope_a())
        .await
        .unwrap();
    assert!(found.ksef_number.is_none());
}

/// `set_ksef_error` with a wrong `account_id` must return NotFound and must not
/// write the error on the invoice belonging to the correct account.
#[tokio::test]
async fn set_ksef_error_wrong_account_returns_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a).await;
    create_account_with_id(&repo, account_id_b(), nip_b).await;

    let mut inv = sample_invoice();
    inv.nip_account_id = account_id_a();
    let id = repo.save(&inv).await.unwrap();

    let err = repo
        .set_ksef_error(&id, &scope_b(), "injected error")
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));

    let found = InvoiceRepository::find_by_id(&repo, &id, &scope_a())
        .await
        .unwrap();
    assert!(found.ksef_error.is_none());
}

/// `find_by_id` with a wrong `account_id` must return NotFound even when the
/// invoice exists in a different account.
#[tokio::test]
async fn find_by_id_wrong_account_returns_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    create_account_with_id(&repo, account_id_a(), nip_a).await;
    create_account_with_id(&repo, account_id_b(), nip_b).await;

    let mut inv = sample_invoice();
    inv.nip_account_id = account_id_a();
    let id = repo.save(&inv).await.unwrap();

    let err = InvoiceRepository::find_by_id(&repo, &id, &scope_b())
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn workspace_members_can_share_single_workspace_account() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let owner = make_user("owner@workspace.pl");
    let owner_id = UserRepository::create(&db, &owner).await.unwrap();
    let operator = make_user("operator@workspace.pl");
    let operator_id = UserRepository::create(&db, &operator).await.unwrap();

    let workspace = WorkspaceRepository::ensure_default_workspace(&db, &owner_id, &owner.email)
        .await
        .unwrap();
    WorkspaceRepository::add_member(
        &db,
        &workspace.workspace.id,
        &operator_id,
        WorkspaceRole::Operator,
    )
    .await
    .unwrap();

    let account = make_nip_account(&test_nip());
    let account_id = NipAccountRepository::create(&db, &account).await.unwrap();
    WorkspaceRepository::attach_nip(
        &db,
        &workspace.workspace.id,
        &account_id,
        ksef_core::domain::workspace::WorkspaceNipOwnership::WorkspaceOwned,
        &owner_id,
    )
    .await
    .unwrap();
    let owner_workspaces = WorkspaceRepository::list_for_user(&db, &owner_id)
        .await
        .unwrap();
    let operator_workspaces = WorkspaceRepository::list_for_user(&db, &operator_id)
        .await
        .unwrap();

    assert_eq!(owner_workspaces.len(), 1);
    assert_eq!(operator_workspaces.len(), 1);
    assert_eq!(
        owner_workspaces[0].workspace.id,
        operator_workspaces[0].workspace.id
    );
    assert_eq!(owner_workspaces[0].membership.role, WorkspaceRole::Owner);
    assert_eq!(
        operator_workspaces[0].membership.role,
        WorkspaceRole::Operator
    );

    let owner_accounts = WorkspaceRepository::list_nip_accounts_for_user(
        &db,
        &owner_workspaces[0].workspace.id,
        &owner_id,
    )
    .await
    .unwrap();
    let operator_accounts = WorkspaceRepository::list_nip_accounts_for_user(
        &db,
        &operator_workspaces[0].workspace.id,
        &operator_id,
    )
    .await
    .unwrap();

    assert_eq!(owner_accounts.len(), 1);
    assert_eq!(operator_accounts.len(), 1);
    assert_eq!(owner_accounts[0].nip, test_nip());
    assert_eq!(operator_accounts[0].nip, test_nip());
}

#[tokio::test]
async fn workspace_lookup_is_scoped_to_active_workspace() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let owner = make_user("owner@scope.pl");
    let owner_id = UserRepository::create(&db, &owner).await.unwrap();
    let outsider = make_user("outsider@scope.pl");
    let outsider_id = UserRepository::create(&db, &outsider).await.unwrap();

    let owner_workspace =
        WorkspaceRepository::ensure_default_workspace(&db, &owner_id, &owner.email)
            .await
            .unwrap();
    let account = make_nip_account(&test_nip());
    let account_id = NipAccountRepository::create(&db, &account).await.unwrap();
    WorkspaceRepository::attach_nip(
        &db,
        &owner_workspace.workspace.id,
        &account_id,
        ksef_core::domain::workspace::WorkspaceNipOwnership::WorkspaceOwned,
        &owner_id,
    )
    .await
    .unwrap();
    let outsider_workspace =
        WorkspaceRepository::ensure_default_workspace(&db, &outsider_id, &outsider.email)
            .await
            .unwrap();

    assert!(
        WorkspaceRepository::find_user_account_in_workspace(
            &db,
            &owner_workspace.workspace.id,
            &owner_id,
            &test_nip(),
        )
        .await
        .unwrap()
        .is_some()
    );
    assert!(
        WorkspaceRepository::find_user_account_in_workspace(
            &db,
            &outsider_workspace.workspace.id,
            &outsider_id,
            &test_nip(),
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn workspace_invite_lifecycle_is_persisted() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let owner = make_user("owner@invite.pl");
    let owner_id = UserRepository::create(&db, &owner).await.unwrap();
    let workspace = WorkspaceRepository::ensure_default_workspace(&db, &owner_id, &owner.email)
        .await
        .unwrap();

    let invite = WorkspaceInvite {
        id: WorkspaceInviteId::new(),
        workspace_id: workspace.workspace.id.clone(),
        email: "new.member@example.com".to_string(),
        role: WorkspaceRole::Operator,
        token_hash: "invite-token-hash".to_string(),
        expires_at: Utc::now() + chrono::Duration::days(3),
        accepted_at: None,
        revoked_at: None,
        created_by_user_id: owner_id.clone(),
        created_at: Utc::now(),
    };

    WorkspaceRepository::create_invite(&db, &invite)
        .await
        .unwrap();
    let pending = WorkspaceRepository::list_pending_invites(&db, &workspace.workspace.id)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].role, WorkspaceRole::Operator);

    let loaded = WorkspaceRepository::find_invite_by_token_hash(&db, &invite.token_hash)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.email, invite.email);

    WorkspaceRepository::accept_invite(&db, &invite.id)
        .await
        .unwrap();
    let accepted = WorkspaceRepository::find_invite_by_token_hash(&db, &invite.token_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(accepted.accepted_at.is_some());

    WorkspaceRepository::revoke_invite(&db, &invite.id)
        .await
        .unwrap();
    let revoked = WorkspaceRepository::find_invite_by_token_hash(&db, &invite.token_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert!(
        WorkspaceRepository::list_pending_invites(&db, &workspace.workspace.id)
            .await
            .unwrap()
            .is_empty()
    );
}
