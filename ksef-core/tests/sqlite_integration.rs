//! Integration tests for SQLite repository and queue implementations.
//!
//! Each test gets its own isolated database file — fully parallel-safe.

use std::str::FromStr;
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
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
use ksef_core::infra::sqlite::{Db, run_migrations};
use ksef_core::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use ksef_core::ports::job_queue::JobQueue;
use ksef_core::ports::nip_account_repository::NipAccountRepository;
use ksef_core::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use ksef_core::ports::user_repository::UserRepository;

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

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    let found = InvoiceRepository::find_by_id(&repo, &id).await.unwrap();
    assert_eq!(found.id.as_uuid(), invoice.id.as_uuid());
    assert_eq!(found.invoice_number, invoice.invoice_number);
    assert_eq!(found.payment_method, Some(PaymentMethod::Transfer));
}

#[tokio::test]
async fn invoice_find_by_id_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let err = InvoiceRepository::find_by_id(&repo, &InvoiceId::new()).await.unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound { .. }));
}

#[tokio::test]
async fn invoice_save_duplicate_returns_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let invoice = sample_invoice();
    repo.save(&invoice).await.unwrap();
    let err = repo.save(&invoice).await.unwrap_err();
    assert!(matches!(err, RepositoryError::Duplicate { .. }));
}

#[tokio::test]
async fn invoice_update_status() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();
    repo.update_status(&id, InvoiceStatus::Queued).await.unwrap();
    let found = InvoiceRepository::find_by_id(&repo, &id).await.unwrap();
    assert_eq!(found.status, InvoiceStatus::Queued);
}

#[tokio::test]
async fn invoice_set_ksef_number() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();
    repo.set_ksef_number(&id, "KSeF-12345").await.unwrap();
    let found = InvoiceRepository::find_by_id(&repo, &id).await.unwrap();
    assert_eq!(found.ksef_number.unwrap().as_str(), "KSeF-12345");
}

#[tokio::test]
async fn invoice_set_ksef_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();
    repo.set_ksef_error(&id, "timeout").await.unwrap();
    let found = InvoiceRepository::find_by_id(&repo, &id).await.unwrap();
    assert_eq!(found.ksef_error.as_deref(), Some("timeout"));
}

#[tokio::test]
async fn invoice_upsert_by_ksef_number_updates_existing() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let mut invoice = sample_invoice();
    invoice.ksef_number = Some(KSeFNumber::new("KSeF-SQLITE-001".to_string()));
    let first_id = repo.upsert_by_ksef_number(&invoice).await.unwrap();

    invoice.invoice_number = "FV/2026/04/002".to_string();
    let second_id = repo.upsert_by_ksef_number(&invoice).await.unwrap();

    assert_eq!(first_id.as_uuid(), second_id.as_uuid());
    let rows = repo
        .list(&InvoiceFilter::for_account(test_nip()).with_direction(Direction::Outgoing))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].invoice_number, "FV/2026/04/002");
}

#[tokio::test]
async fn invoice_list_filters_by_direction() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let mut outgoing = sample_invoice();
    outgoing.direction = Direction::Outgoing;
    repo.save(&outgoing).await.unwrap();

    let mut incoming = sample_invoice();
    incoming.direction = Direction::Incoming;
    repo.save(&incoming).await.unwrap();

    let result = repo
        .list(&InvoiceFilter::for_account(test_nip()).with_direction(Direction::Outgoing))
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Outgoing);
}

#[tokio::test]
async fn invoice_list_filters_by_status() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let inv = sample_invoice();
    let id = repo.save(&inv).await.unwrap();
    repo.update_status(&id, InvoiceStatus::Queued).await.unwrap();

    let inv2 = sample_invoice();
    repo.save(&inv2).await.unwrap(); // stays Draft

    let result = repo
        .list(&InvoiceFilter::for_account(test_nip()).with_status(InvoiceStatus::Queued))
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
}

// ===========================================================================
// Invoice Filter: account_nip tenant isolation
// ===========================================================================

#[tokio::test]
async fn invoice_account_nip_filters_by_seller_or_buyer() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    let nip_c = Nip::parse("7740001454").unwrap();

    // Invoice 1: A sells to B
    let mut inv1 = sample_invoice();
    inv1.direction = Direction::Outgoing;
    inv1.seller.nip = Some(nip_a.clone());
    inv1.buyer.nip = Some(nip_b.clone());
    repo.save(&inv1).await.unwrap();

    // Invoice 2: C sells to A (A is buyer)
    let mut inv2 = sample_invoice();
    inv2.direction = Direction::Incoming;
    inv2.seller.nip = Some(nip_c.clone());
    inv2.buyer.nip = Some(nip_a.clone());
    repo.save(&inv2).await.unwrap();

    // Invoice 3: B sells to C (A is not involved)
    let mut inv3 = sample_invoice();
    inv3.direction = Direction::Outgoing;
    inv3.seller.nip = Some(nip_b.clone());
    inv3.buyer.nip = Some(nip_c.clone());
    repo.save(&inv3).await.unwrap();

    // Filter by account_nip = A → should see inv1 (seller) and inv2 (buyer), NOT inv3
    let result = repo
        .list(&InvoiceFilter::for_account(nip_a.clone()))
        .await
        .unwrap();
    assert_eq!(result.len(), 2);

    // Filter by account_nip = C → should see inv2 (seller) and inv3 (buyer)
    let result = repo
        .list(&InvoiceFilter::for_account(nip_c))
        .await
        .unwrap();
    assert_eq!(result.len(), 2);

    // Filter by account_nip = B → should see inv1 (buyer) and inv3 (seller)
    let result = repo
        .list(&InvoiceFilter::for_account(nip_b))
        .await
        .unwrap();
    assert_eq!(result.len(), 2);
}

#[tokio::test]
async fn invoice_account_nip_combined_with_direction() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();

    // Outgoing: A sells to B
    let mut inv1 = sample_invoice();
    inv1.direction = Direction::Outgoing;
    inv1.seller.nip = Some(nip_a.clone());
    inv1.buyer.nip = Some(nip_b.clone());
    repo.save(&inv1).await.unwrap();

    // Incoming: B sells to A
    let mut inv2 = sample_invoice();
    inv2.direction = Direction::Incoming;
    inv2.seller.nip = Some(nip_b.clone());
    inv2.buyer.nip = Some(nip_a.clone());
    repo.save(&inv2).await.unwrap();

    // account_nip=A + direction=Outgoing → only inv1
    let result = repo
        .list(&InvoiceFilter::for_account(nip_a.clone()).with_direction(Direction::Outgoing))
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Outgoing);

    // account_nip=A + direction=Incoming → only inv2
    let result = repo
        .list(&InvoiceFilter::for_account(nip_a).with_direction(Direction::Incoming))
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, Direction::Incoming);
}

#[tokio::test]
async fn invoice_account_nip_returns_empty_for_uninvolved_nip() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let nip_a = Nip::parse("5260250274").unwrap();
    let nip_b = Nip::parse("1060000062").unwrap();
    let nip_unrelated = Nip::parse("7740001454").unwrap();

    let mut inv = sample_invoice();
    inv.seller.nip = Some(nip_a);
    inv.buyer.nip = Some(nip_b);
    repo.save(&inv).await.unwrap();

    let result = repo
        .list(&InvoiceFilter::for_account(nip_unrelated))
        .await
        .unwrap();
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

    let found = UserRepository::find_by_email(&repo, "bob@example.com").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().email, "bob@example.com");
}

#[tokio::test]
async fn user_find_by_email_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let found = UserRepository::find_by_email(&repo, "nobody@example.com").await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn user_find_by_id_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let err = UserRepository::find_by_id(&repo, &UserId::new()).await.unwrap_err();
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

    UserRepository::create(&repo, &make_user("a@example.com")).await.unwrap();
    UserRepository::create(&repo, &make_user("b@example.com")).await.unwrap();
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

    let found = NipAccountRepository::find_by_nip(&repo, &test_nip()).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().nip.as_str(), "5260250274");
}

#[tokio::test]
async fn nip_account_find_by_nip_not_found() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);
    let found = NipAccountRepository::find_by_nip(&repo, &other_nip()).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn nip_account_duplicate_nip_returns_error() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let acc1 = make_nip_account(&test_nip());
    NipAccountRepository::create(&repo, &acc1).await.unwrap();

    let acc2 = make_nip_account(&test_nip());
    let err = NipAccountRepository::create(&repo, &acc2).await.unwrap_err();
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
    account.cert_pem = Some(b"-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----".to_vec());
    account.key_pem = Some(b"-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----".to_vec());
    account.cert_auto_generated = true;
    NipAccountRepository::update_credentials(&repo, &account).await.unwrap();

    let found = NipAccountRepository::find_by_nip(&repo, &test_nip()).await.unwrap().unwrap();
    assert_eq!(found.ksef_auth_method, KSeFAuthMethod::Token);
    assert_eq!(found.ksef_auth_token.as_deref(), Some("my-token"));
    assert!(found.cert_pem.is_some());
    assert!(found.key_pem.is_some());
    assert!(found.cert_auto_generated);
}

// ===========================================================================
// User <-> NIP Account access control
// ===========================================================================

#[tokio::test]
async fn access_grant_and_list_by_user() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("owner@firm.pl");
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
    assert_eq!(result.unwrap().nip.as_str(), "5260250274");
}

#[tokio::test]
async fn access_has_access_returns_none_when_not_granted() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("u@x.pl");
    let user_id = UserRepository::create(&db, &user).await.unwrap();

    // Account exists but user has no access
    let acc = make_nip_account(&test_nip());
    NipAccountRepository::create(&db, &acc).await.unwrap();

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

    // Verify access exists
    assert!(db.has_access(&user_id, &test_nip()).await.unwrap().is_some());

    // Revoke
    db.revoke_access(&user_id, &acc_id).await.unwrap();

    // Verify access gone
    assert!(db.has_access(&user_id, &test_nip()).await.unwrap().is_none());
    assert!(db.list_by_user(&user_id).await.unwrap().is_empty());
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

    // Only Alice gets access
    db.grant_access(&alice_id, &acc_id).await.unwrap();

    assert!(db.has_access(&alice_id, &test_nip()).await.unwrap().is_some());
    assert!(db.has_access(&bob_id, &test_nip()).await.unwrap().is_none());

    assert_eq!(db.list_by_user(&alice_id).await.unwrap().len(), 1);
    assert_eq!(db.list_by_user(&bob_id).await.unwrap().len(), 0);
}

#[tokio::test]
async fn access_multiple_users_same_nip() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let alice = make_user("alice@x.pl");
    let alice_id = UserRepository::create(&db, &alice).await.unwrap();
    let bob = make_user("bob@x.pl");
    let bob_id = UserRepository::create(&db, &bob).await.unwrap();

    let acc = make_nip_account(&test_nip());
    let acc_id = NipAccountRepository::create(&db, &acc).await.unwrap();

    db.grant_access(&alice_id, &acc_id).await.unwrap();
    db.grant_access(&bob_id, &acc_id).await.unwrap();

    // Both have access
    assert!(db.has_access(&alice_id, &test_nip()).await.unwrap().is_some());
    assert!(db.has_access(&bob_id, &test_nip()).await.unwrap().is_some());
}

#[tokio::test]
async fn access_list_empty_for_new_user() {
    let pool = isolated_pool().await;
    let db = Db::new(pool);

    let user = make_user("new@x.pl");
    let user_id = UserRepository::create(&db, &user).await.unwrap();

    assert!(db.list_by_user(&user_id).await.unwrap().is_empty());
}
