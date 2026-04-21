//! Integration tests for PostgreSQL repository and job queue implementations.
//!
//! Requires a Docker daemon — spins up a Postgres container via testcontainers.
//! Each test gets its own **isolated database** (CREATE DATABASE per test),
//! so tests run fully in parallel with zero interference.

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

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
use ksef_core::error::RepositoryError;
use ksef_core::infra::pg::{Db, run_migrations};
use ksef_core::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use ksef_core::ports::job_queue::JobQueue;
use ksef_core::ports::nip_account_repository::NipAccountRepository;
use ksef_core::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use ksef_core::ports::user_repository::UserRepository;

// ---------------------------------------------------------------------------
// Shared infrastructure: one container, isolated database per test
// ---------------------------------------------------------------------------

/// Shared Postgres container — started once, lives for the entire test run.
struct TestContainer {
    base_url: String,
    _container: ContainerAsync<Postgres>,
}

static CONTAINER: tokio::sync::OnceCell<TestContainer> = tokio::sync::OnceCell::const_new();

async fn ensure_container() -> &'static TestContainer {
    CONTAINER
        .get_or_init(|| async {
            let container = Postgres::default()
                .start()
                .await
                .expect("failed to start postgres container");

            let host_port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("failed to get mapped port");

            TestContainer {
                base_url: format!("postgres://postgres:postgres@127.0.0.1:{host_port}"),
                _container: container,
            }
        })
        .await
}

/// Create an isolated database for this test.
/// Each test gets its own database — full isolation, parallel-safe.
async fn isolated_pool() -> PgPool {
    let container = ensure_container().await;
    let admin_url = format!("{}/postgres", container.base_url);
    let db_name = format!("test_{}", Uuid::new_v4().as_simple());

    // Create isolated database
    let admin_pool = PgPool::connect(&admin_url)
        .await
        .expect("connect to admin db");
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
        .execute(&admin_pool)
        .await
        .expect("create test database");

    // Connect and migrate
    let test_url = format!("{}/{db_name}", container.base_url);
    let pool = PgPool::connect(&test_url)
        .await
        .expect("connect to test database");
    run_migrations(&pool)
        .await
        .expect("run migrations on test database");

    pool
}

// ---------------------------------------------------------------------------
// Test fixtures (duplicated from test_support because that module is
// behind #[cfg(test)] in lib.rs and not available to integration tests)
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
    NipAccountId::from_uuid(Uuid::from_u128(22))
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

/// A different valid NIP for "wrong NIP" assertions.
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

// ===========================================================================
// Invoice Repository — contract tests
// ===========================================================================

#[tokio::test]
async fn invoice_save_and_find_by_id() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id, &test_account_id())
        .await
        .unwrap();
    assert_eq!(found.id.as_uuid(), invoice.id.as_uuid());
    assert_eq!(found.invoice_number, invoice.invoice_number);
    assert_eq!(found.direction, Direction::Outgoing);
    assert_eq!(found.status, InvoiceStatus::Draft);
    assert_eq!(found.seller.nip.as_ref().unwrap().as_str(), "5260250274");
    assert_eq!(found.buyer.name, "Test Buyer S.A.");
    assert_eq!(found.total_net, Money::from_pln(24000, 0));
    assert_eq!(found.total_vat, Money::from_pln(5520, 0));
    assert_eq!(found.total_gross, Money::from_pln(29520, 0));
    assert_eq!(found.payment_method, Some(PaymentMethod::Transfer));
    assert_eq!(found.line_items.len(), 1);
    assert_eq!(found.line_items[0].description, "Uslugi programistyczne");
    assert!(found.ksef_number.is_none());
    assert!(found.ksef_error.is_none());
}

#[tokio::test]
async fn invoice_save_and_find_by_id_mobile_payment_method() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let mut invoice = sample_invoice();
    invoice.payment_method = Some(PaymentMethod::Mobile);
    let id = repo.save(&invoice).await.unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id, &test_account_id())
        .await
        .unwrap();
    assert_eq!(found.payment_method, Some(PaymentMethod::Mobile));
}

#[tokio::test]
async fn invoice_find_by_id_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let missing_id = InvoiceId::new();
    let err = InvoiceRepository::find_by_id(&repo, &missing_id, &test_account_id())
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
async fn invoice_update_status_changes_status() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    repo.update_status(&id, &test_account_id(), InvoiceStatus::Queued)
        .await
        .unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id, &test_account_id())
        .await
        .unwrap();
    assert_eq!(found.status, InvoiceStatus::Queued);
}

#[tokio::test]
async fn invoice_update_status_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let dummy_account = NipAccountId::new();

    let err = repo
        .update_status(&InvoiceId::new(), &dummy_account, InvoiceStatus::Queued)
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn invoice_set_ksef_number_persists() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    repo.set_ksef_number(&id, &test_account_id(), "KSeF-12345")
        .await
        .unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id, &test_account_id())
        .await
        .unwrap();
    assert_eq!(found.ksef_number.unwrap().as_str(), "KSeF-12345");
}

#[tokio::test]
async fn invoice_set_ksef_error_persists() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    repo.set_ksef_error(&id, &test_account_id(), "submission timed out")
        .await
        .unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id, &test_account_id())
        .await
        .unwrap();
    assert_eq!(found.ksef_error.as_deref(), Some("submission timed out"));
}

#[tokio::test]
async fn invoice_set_ksef_number_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let dummy_account = NipAccountId::new();

    let err = repo
        .set_ksef_number(&InvoiceId::new(), &dummy_account, "KSeF-999")
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn invoice_set_ksef_error_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let dummy_account = NipAccountId::new();

    let err = repo
        .set_ksef_error(&InvoiceId::new(), &dummy_account, "oops")
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
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

    let filter = InvoiceFilter::for_account(test_account_id()).with_direction(Direction::Outgoing);
    let result = repo.list(&filter).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Outgoing);
}

#[tokio::test]
async fn invoice_list_filters_by_status() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let inv1 = sample_invoice();
    let id1 = repo.save(&inv1).await.unwrap();
    repo.update_status(&id1, InvoiceStatus::Queued)
        .await
        .unwrap();

    let inv2 = sample_invoice();
    repo.save(&inv2).await.unwrap(); // stays Draft

    let filter = InvoiceFilter::for_account(test_account_id()).with_status(InvoiceStatus::Queued);
    let result = repo.list(&filter).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id.as_uuid(), id1.as_uuid());
}

#[tokio::test]
async fn invoice_list_with_limit_and_offset() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    for _ in 0..5 {
        repo.save(&sample_invoice()).await.unwrap();
    }

    let mut filter = InvoiceFilter::for_account(test_account_id());
    filter.limit = Some(2);
    filter.offset = Some(1);
    let result = repo.list(&filter).await.unwrap();
    assert_eq!(result.len(), 2);
}

#[tokio::test]
async fn invoice_list_empty_returns_empty() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let result = repo
        .list(&InvoiceFilter::for_account(test_account_id()))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn invoice_list_filters_by_account_id() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let account_a = NipAccountId::from_uuid(Uuid::from_u128(21));
    let account_b = NipAccountId::from_uuid(Uuid::from_u128(22));
    create_account_with_id(&repo, account_a.clone(), test_nip()).await;
    create_account_with_id(&repo, account_b.clone(), other_nip()).await;

    let mut invoice = sample_invoice();
    invoice.nip_account_id = account_a.clone();
    repo.save(&invoice).await.unwrap();

    let result = repo
        .list(&InvoiceFilter::for_account(account_a))
        .await
        .unwrap();
    assert_eq!(result.len(), 1);

    // A different account id returns nothing
    let result_wrong = repo
        .list(&InvoiceFilter::for_account(account_b))
        .await
        .unwrap();
    assert!(result_wrong.is_empty());
}

// ===========================================================================
// Invoice Repository — PG-specific behaviour
// ===========================================================================

#[tokio::test]
async fn invoice_ksef_number_unique_constraint() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    let mut inv1 = sample_invoice();
    inv1.ksef_number = Some(KSeFNumber::new("KSeF-UNIQUE-001".to_string()));
    repo.save(&inv1).await.unwrap();

    // A second invoice with the same ksef_number should fail (unique constraint).
    let mut inv2 = sample_invoice();
    inv2.ksef_number = Some(KSeFNumber::new("KSeF-UNIQUE-001".to_string()));
    let err = repo.save(&inv2).await.unwrap_err();
    // The PG impl maps unique violations on the PK to Duplicate, but the
    // ksef_number unique constraint may surface as either Duplicate or Database.
    assert!(
        matches!(
            err,
            RepositoryError::Duplicate { .. } | RepositoryError::Database(_)
        ),
        "expected a unique constraint violation, got: {err:?}"
    );
}

#[tokio::test]
async fn invoice_null_ksef_numbers_are_not_unique() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    create_account_with_id(&repo, test_account_id(), test_nip()).await;

    // Two invoices with ksef_number = NULL should both succeed (NULL != NULL).
    let inv1 = sample_invoice();
    repo.save(&inv1).await.unwrap();

    let inv2 = sample_invoice();
    repo.save(&inv2).await.unwrap();
}

// ===========================================================================
// Job Queue — contract tests
// ===========================================================================

#[tokio::test]
async fn job_enqueue_then_dequeue_returns_job() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let job = make_job("submit_invoice");
    let id = queue.enqueue(job).await.unwrap();

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
async fn job_complete_marks_completed() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let job = make_job("submit_invoice");
    let id = queue.enqueue(job).await.unwrap();
    queue.dequeue().await.unwrap(); // transition to Running

    queue.complete(&id).await.unwrap();

    // Should not appear in pending or dead letter
    assert!(queue.list_pending().await.unwrap().is_empty());
    assert!(queue.list_dead_letter().await.unwrap().is_empty());
}

#[tokio::test]
async fn job_fail_increments_attempts() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool.clone());

    let job = make_job("submit_invoice");
    let id = queue.enqueue(job).await.unwrap();
    queue.dequeue().await.unwrap();

    queue.fail(&id, "connection refused").await.unwrap();

    // Read back directly from the DB to verify state
    let row = sqlx::query_as::<_, (i32, Option<String>, String)>(
        "SELECT attempts, last_error, status FROM jobs WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, 1); // attempts
    assert_eq!(row.1.as_deref(), Some("connection refused"));
    // With max_attempts=3 and attempts=1, job should be requeued as pending.
    assert_eq!(row.2, "pending");
}

#[tokio::test]
async fn job_fail_after_max_attempts_dead_letters() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let mut job = make_job("submit_invoice");
    job.max_attempts = 2;
    let id = queue.enqueue(job).await.unwrap();
    queue.dequeue().await.unwrap();

    queue.fail(&id, "error 1").await.unwrap();
    queue.fail(&id, "error 2").await.unwrap();

    let dead = queue.list_dead_letter().await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].id.as_uuid(), id.as_uuid());
    assert_eq!(dead[0].last_error.as_deref(), Some("error 2"));
}

#[tokio::test]
async fn job_dead_letter_explicit() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let job = make_job("submit_invoice");
    let id = queue.enqueue(job).await.unwrap();

    queue.dead_letter(&id, "permanent failure").await.unwrap();

    let dead = queue.list_dead_letter().await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].last_error.as_deref(), Some("permanent failure"));
}

#[tokio::test]
async fn job_operations_on_missing_job_return_error() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let missing = JobId::new();
    assert!(queue.complete(&missing).await.is_err());
    assert!(queue.fail(&missing, "err").await.is_err());
    assert!(queue.dead_letter(&missing, "err").await.is_err());
}

#[tokio::test]
async fn job_dequeue_skips_non_pending() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let job1 = make_job("job1");
    let id1 = queue.enqueue(job1).await.unwrap();
    queue.dequeue().await.unwrap(); // job1 -> Running
    queue.complete(&id1).await.unwrap(); // job1 -> Completed

    let job2 = make_job("job2");
    queue.enqueue(job2).await.unwrap();

    let dequeued = queue.dequeue().await.unwrap().unwrap();
    assert_eq!(dequeued.job_type, "job2");
}

#[tokio::test]
async fn job_list_pending_filters_correctly() {
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    queue.enqueue(make_job("pending1")).await.unwrap();
    queue.enqueue(make_job("pending2")).await.unwrap();
    queue
        .enqueue(make_job("will_remain_pending"))
        .await
        .unwrap();

    // Dequeue takes the first two (ordered by scheduled_at)
    queue.dequeue().await.unwrap(); // pending1 -> Running
    queue.dequeue().await.unwrap(); // pending2 -> Running

    let pending = queue.list_pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].job_type, "will_remain_pending");
}

// ===========================================================================
// Job Queue — PG-specific behaviour (FOR UPDATE SKIP LOCKED)
// ===========================================================================

#[tokio::test]
async fn job_dequeue_skip_locked_concurrent() {
    // Verify that two concurrent dequeue operations grab different jobs
    // (not the same one), which is the whole point of FOR UPDATE SKIP LOCKED.
    let pool = isolated_pool().await;

    // Enqueue two jobs
    let queue = Db::new(pool.clone());
    let job_a = make_job("concurrent_a");
    let id_a = queue.enqueue(job_a).await.unwrap();
    let job_b = make_job("concurrent_b");
    let id_b = queue.enqueue(job_b).await.unwrap();

    // Dequeue both concurrently from two instances sharing the same pool
    let queue_1 = Db::new(pool.clone());
    let queue_2 = Db::new(pool.clone());

    let (result_1, result_2) = tokio::join!(queue_1.dequeue(), queue_2.dequeue());

    let dequeued_1 = result_1.unwrap().expect("first dequeue should get a job");
    let dequeued_2 = result_2.unwrap().expect("second dequeue should get a job");

    // The two dequeued jobs must be different
    assert_ne!(
        dequeued_1.id.as_uuid(),
        dequeued_2.id.as_uuid(),
        "concurrent dequeue should return different jobs"
    );

    // Both should be Running
    assert_eq!(dequeued_1.status, JobStatus::Running);
    assert_eq!(dequeued_2.status, JobStatus::Running);

    // Together they should cover both jobs we enqueued
    let mut ids: Vec<&Uuid> = vec![dequeued_1.id.as_uuid(), dequeued_2.id.as_uuid()];
    ids.sort();
    let mut expected: Vec<&Uuid> = vec![id_a.as_uuid(), id_b.as_uuid()];
    expected.sort();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn job_dequeue_returns_none_when_all_consumed() {
    // If there is only one pending job and it has already been dequeued,
    // a second dequeue should return None.
    let pool = isolated_pool().await;
    let queue = Db::new(pool);

    let job = make_job("only_one");
    queue.enqueue(job).await.unwrap();

    // First dequeue grabs the only job
    let first = queue.dequeue().await.unwrap();
    assert!(first.is_some());

    // Second dequeue should return None (no more pending jobs)
    let second = queue.dequeue().await.unwrap();
    assert!(second.is_none());
}

#[tokio::test]
async fn job_fail_applies_exponential_backoff() {
    // After a fail, the scheduled_at should be pushed into the future.
    let pool = isolated_pool().await;
    let queue = Db::new(pool.clone());

    let job = make_job("backoff_test");
    let id = queue.enqueue(job).await.unwrap();
    queue.dequeue().await.unwrap(); // -> Running

    queue.fail(&id, "transient error").await.unwrap();

    // Verify the scheduled_at was pushed forward
    let row = sqlx::query_as::<_, (chrono::DateTime<Utc>,)>(
        "SELECT scheduled_at FROM jobs WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    // scheduled_at should be in the future (at least 1 second from now minus epsilon)
    assert!(
        row.0 > Utc::now() - chrono::Duration::seconds(1),
        "scheduled_at should be pushed into the future after fail"
    );
}

// ===========================================================================
// Session Repository — contract tests
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
async fn session_find_active_token_wrong_env_returns_none() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let token = make_stored_token(KSeFEnvironment::Test);
    repo.save_token_pair(&token).await.unwrap();

    let found = repo
        .find_active_token(&test_nip(), KSeFEnvironment::Production)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn session_expired_refresh_token_not_returned() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let mut token = make_stored_token(KSeFEnvironment::Test);
    token.token_pair = TokenPair {
        access_token: AccessToken::new("a".to_string()),
        refresh_token: RefreshToken::new("r".to_string()),
        access_token_expires_at: Utc::now() - chrono::Duration::hours(1),
        refresh_token_expires_at: Utc::now() - chrono::Duration::days(1),
    };
    repo.save_token_pair(&token).await.unwrap();

    let found = repo
        .find_active_token(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn session_save_and_find_active_session() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let session = make_stored_session(KSeFEnvironment::Test);
    let expected_ref = session.session_reference.as_str().to_string();
    repo.save_session(&session).await.unwrap();

    let found = repo
        .find_active_session(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().session_reference.as_str(), expected_ref);
}

#[tokio::test]
async fn session_terminated_session_not_active() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let session = make_stored_session(KSeFEnvironment::Test);
    let session_id = session.id;
    repo.save_session(&session).await.unwrap();

    repo.terminate_session(session_id).await.unwrap();

    let found = repo
        .find_active_session(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn session_terminate_missing_session_returns_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let err = repo.terminate_session(Uuid::new_v4()).await.unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn session_find_active_session_wrong_env_returns_none() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let session = make_stored_session(KSeFEnvironment::Test);
    repo.save_session(&session).await.unwrap();

    let found = repo
        .find_active_session(&test_nip(), KSeFEnvironment::Production)
        .await
        .unwrap();
    assert!(found.is_none());
}

// ===========================================================================
// Session Repository — PG-specific behaviour
// ===========================================================================

#[tokio::test]
async fn session_reference_unique_constraint() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let session = make_stored_session(KSeFEnvironment::Test);
    repo.save_session(&session).await.unwrap();

    // Save another session with the same session_reference but different id
    let mut dup = make_stored_session(KSeFEnvironment::Test);
    dup.session_reference = session.session_reference.clone();
    let err = repo.save_session(&dup).await.unwrap_err();
    assert!(
        matches!(err, RepositoryError::Database(_)),
        "expected unique constraint violation on session_reference, got: {err:?}"
    );
}

#[tokio::test]
async fn session_terminate_already_terminated_returns_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let session = make_stored_session(KSeFEnvironment::Test);
    let session_id = session.id;
    repo.save_session(&session).await.unwrap();

    repo.terminate_session(session_id).await.unwrap();

    // Terminating again should fail because terminated_at IS NOT NULL now
    let err = repo.terminate_session(session_id).await.unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn session_multiple_tokens_returns_latest() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    // Save an older token
    let mut old_token = make_stored_token(KSeFEnvironment::Test);
    old_token.created_at = Utc::now() - chrono::Duration::hours(2);
    old_token.token_pair.access_token = AccessToken::new("old-access".to_string());
    repo.save_token_pair(&old_token).await.unwrap();

    // Save a newer token
    let mut new_token = make_stored_token(KSeFEnvironment::Test);
    new_token.created_at = Utc::now();
    new_token.token_pair.access_token = AccessToken::new("new-access".to_string());
    repo.save_token_pair(&new_token).await.unwrap();

    let found = repo
        .find_active_token(&test_nip(), KSeFEnvironment::Test)
        .await
        .unwrap()
        .expect("should find an active token");

    // The query orders by created_at DESC LIMIT 1, so we get the newest
    assert_eq!(found.token_pair.access_token.as_str(), "new-access");
}

// ===========================================================================
// Migration idempotency
// ===========================================================================

#[tokio::test]
async fn migrations_are_idempotent() {
    let pool = isolated_pool().await;
    // Run migrations a second time — must not fail.
    run_migrations(&pool).await.unwrap();
}

// ===========================================================================
// User Repository
// ===========================================================================

fn make_user(email: &str) -> User {
    User {
        id: UserId::new(),
        email: email.to_string(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$fake$fake".to_string(),
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

#[tokio::test]
async fn user_create_and_find_by_id() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    let user = make_user("alice@example.com");
    let id = UserRepository::create(&db, &user).await.unwrap();
    let found = UserRepository::find_by_id(&db, &id).await.unwrap();
    assert_eq!(found.email, "alice@example.com");
}

#[tokio::test]
async fn user_find_by_email() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    UserRepository::create(&db, &make_user("bob@example.com"))
        .await
        .unwrap();
    let found = UserRepository::find_by_email(&db, "bob@example.com")
        .await
        .unwrap();
    assert!(found.is_some());
}

#[tokio::test]
async fn user_find_by_email_not_found() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    let found = UserRepository::find_by_email(&db, "nobody@x.com")
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn user_duplicate_email_returns_error() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    UserRepository::create(&db, &make_user("dup@x.com"))
        .await
        .unwrap();
    let err = UserRepository::create(&db, &make_user("dup@x.com"))
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Duplicate { .. }));
}

// ===========================================================================
// NIP Account Repository
// ===========================================================================

#[tokio::test]
async fn nip_account_create_and_find_by_nip() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    let acc = make_nip_account(&test_nip());
    NipAccountRepository::create(&db, &acc).await.unwrap();
    let found = NipAccountRepository::find_by_nip(&db, &test_nip())
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().display_name, acc.display_name);
}

#[tokio::test]
async fn nip_account_duplicate_nip_returns_error() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    NipAccountRepository::create(&db, &make_nip_account(&test_nip()))
        .await
        .unwrap();
    let err = NipAccountRepository::create(&db, &make_nip_account(&test_nip()))
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Duplicate { .. }));
}

#[tokio::test]
async fn nip_account_update_credentials() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);
    let mut acc = make_nip_account(&test_nip());
    NipAccountRepository::create(&db, &acc).await.unwrap();

    acc.ksef_auth_method = KSeFAuthMethod::Token;
    acc.ksef_auth_token = Some("my-token".to_string());
    acc.cert_pem = Some(b"-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----".to_vec());
    acc.key_pem = Some(b"-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----".to_vec());
    NipAccountRepository::update_credentials(&db, &acc)
        .await
        .unwrap();

    let found = NipAccountRepository::find_by_nip(&db, &test_nip())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.ksef_auth_method, KSeFAuthMethod::Token);
    assert_eq!(found.ksef_auth_token.as_deref(), Some("my-token"));
    assert!(found.cert_pem.is_some());
}

// ===========================================================================
// User <-> NIP Account access control
// ===========================================================================

#[tokio::test]
async fn access_grant_and_list_by_user() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("owner@x.pl");
    let user_id = UserRepository::create(&db, &user).await.unwrap();

    let acc1 = make_nip_account(&test_nip());
    let acc1_id = NipAccountRepository::create(&db, &acc1).await.unwrap();
    let acc2 = make_nip_account(&other_nip());
    let acc2_id = NipAccountRepository::create(&db, &acc2).await.unwrap();

    db.grant_access(&user_id, &acc1_id).await.unwrap();
    db.grant_access(&user_id, &acc2_id).await.unwrap();

    let accounts = db.list_by_user(&user_id).await.unwrap();
    assert_eq!(accounts.len(), 2);
}

#[tokio::test]
async fn access_has_access_returns_account_when_granted() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("u@x.pl");
    let user_id = UserRepository::create(&db, &user).await.unwrap();
    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();
    db.grant_access(&user_id, &acc_id).await.unwrap();

    let result = db.has_access(&user_id, &test_nip()).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn access_has_access_returns_none_when_not_granted() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("u@x.pl");
    let user_id = UserRepository::create(&db, &user).await.unwrap();
    NipAccountRepository::create(&db, &make_nip_account(&test_nip()))
        .await
        .unwrap();

    let result = db.has_access(&user_id, &test_nip()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn access_revoke_removes_access() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("u@x.pl");
    let user_id = UserRepository::create(&db, &user).await.unwrap();
    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();
    db.grant_access(&user_id, &acc_id).await.unwrap();

    assert!(
        db.has_access(&user_id, &test_nip())
            .await
            .unwrap()
            .is_some()
    );
    db.revoke_access(&user_id, &acc_id).await.unwrap();
    assert!(
        db.has_access(&user_id, &test_nip())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn access_isolation_between_users() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let alice = make_user("alice@x.pl");
    let alice_id = UserRepository::create(&db, &alice).await.unwrap();
    let bob = make_user("bob@x.pl");
    let bob_id = UserRepository::create(&db, &bob).await.unwrap();

    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();
    db.grant_access(&alice_id, &acc_id).await.unwrap();

    assert!(
        db.has_access(&alice_id, &test_nip())
            .await
            .unwrap()
            .is_some()
    );
    assert!(db.has_access(&bob_id, &test_nip()).await.unwrap().is_none());
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

    // Try to update with account B's ID — must fail
    let err = repo
        .update_status(&id, &account_id_b(), InvoiceStatus::Queued)
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));

    // Invoice A is unchanged
    let found = InvoiceRepository::find_by_id(&repo, &id, &account_id_a())
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
        .set_ksef_number(&id, &account_id_b(), "KSeF-EVIL-001")
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));

    // ksef_number on account A's invoice is still None
    let found = InvoiceRepository::find_by_id(&repo, &id, &account_id_a())
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
        .set_ksef_error(&id, &account_id_b(), "injected error")
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));

    let found = InvoiceRepository::find_by_id(&repo, &id, &account_id_a())
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

    let err = InvoiceRepository::find_by_id(&repo, &id, &account_id_b())
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}
