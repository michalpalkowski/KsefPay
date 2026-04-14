//! Integration tests for SQLite repository and queue implementations.

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
use ksef_core::domain::session::{KSeFNumber, SessionReference};
use ksef_core::infra::sqlite::{Db, run_migrations};
use ksef_core::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};
use ksef_core::ports::job_queue::JobQueue;
use ksef_core::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};

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

#[tokio::test]
async fn invoice_save_and_find_by_id() {
    let pool = isolated_pool().await;
    let repo = Db::new(pool);

    let invoice = sample_invoice();
    let id = repo.save(&invoice).await.unwrap();

    let found = repo.find_by_id(&id).await.unwrap();
    assert_eq!(found.id.as_uuid(), invoice.id.as_uuid());
    assert_eq!(found.invoice_number, invoice.invoice_number);
    assert_eq!(found.payment_method, Some(PaymentMethod::Transfer));
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
        .list(&InvoiceFilter {
            direction: Some(Direction::Outgoing),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].invoice_number, "FV/2026/04/002");
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
