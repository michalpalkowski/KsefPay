//! End-to-end test against the real KSeF test environment.
//!
//! This test validates the full flow: auth → session → send invoice → close.
//! Requires network access to `api-test.ksef.mf.gov.pl`.
//!
//! Run with:
//! ```sh
//! cargo test -p ksef-core --test ksef_e2e -- --ignored --nocapture --test-threads=1
//! ```

use ksef_core::domain::auth::AuthStatus;
use ksef_core::domain::batch::PartUploadRequest;
use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::invoice::Direction;
use ksef_core::domain::nip::Nip;
use ksef_core::domain::permission::{
    PermissionGrantRequest, PermissionRevokeRequest, PermissionType,
};
use ksef_core::domain::session::{InvoiceQuery, SubjectType};
use ksef_core::domain::xml::InvoiceXml;
use ksef_core::infra::batch::zip_builder::BatchFileBuilder;
use ksef_core::infra::crypto::AesCbcEncryptor;
use ksef_core::infra::crypto::OpenSslXadesSigner;
use ksef_core::infra::fa3::{invoice_to_xml, xml_to_invoice};
use ksef_core::infra::ksef::KSeFApiClient;
use ksef_core::ports::encryption::{InvoiceEncryptor, XadesSigner};
use ksef_core::ports::ksef_auth::KSeFAuth;
use ksef_core::ports::ksef_auth_sessions::KSeFAuthSessions;
use ksef_core::ports::ksef_batch::{BatchOpenRequest, KSeFBatch};
use ksef_core::ports::ksef_certificates::{CertificateQueryRequest, KSeFCertificates};
use ksef_core::ports::ksef_client::KSeFClient;
use ksef_core::ports::ksef_export::{ExportRequest, KSeFExport};
use ksef_core::ports::ksef_peppol::{KSeFPeppol, PeppolQueryRequest};
use ksef_core::ports::ksef_permissions::{KSeFPermissions, PermissionQueryRequest};
use ksef_core::ports::ksef_rate_limits::KSeFRateLimits;
use ksef_core::ports::ksef_tokens::{KSeFTokens, TokenGenerateRequest, TokenQueryRequest};

fn e2e_environment() -> KSeFEnvironment {
    std::env::var("KSEF_E2E_ENV")
        .ok()
        .and_then(|raw| raw.parse::<KSeFEnvironment>().ok())
        .unwrap_or(KSeFEnvironment::Test)
}

fn e2e_nip() -> Nip {
    let raw = std::env::var("KSEF_E2E_NIP").unwrap_or_else(|_| "5260250274".to_string());
    Nip::parse(&raw).unwrap_or_else(|e| panic!("invalid KSEF_E2E_NIP '{raw}': {e}"))
}

fn read_pem_env(var: &str) -> Result<Vec<u8>, String> {
    let raw = std::env::var(var).map_err(|_| format!("{var} is not set"))?;
    let path = std::path::Path::new(&raw);
    if !raw.contains("-----BEGIN") && path.exists() {
        return std::fs::read(path)
            .map_err(|e| format!("{var} points to unreadable file '{raw}': {e}"));
    }

    let normalized = raw.replace("\\n", "\n");
    if !normalized.contains("-----BEGIN") {
        return Err(format!(
            "{var} must contain PEM content or a path to a PEM file"
        ));
    }

    Ok(normalized.into_bytes())
}

fn load_e2e_signer_from_env() -> OpenSslXadesSigner {
    let cert_present = std::env::var_os("KSEF_E2E_CERT_PEM").is_some();
    let key_present = std::env::var_os("KSEF_E2E_KEY_PEM").is_some();

    match (cert_present, key_present) {
        (true, true) => {
            let cert_pem = read_pem_env("KSEF_E2E_CERT_PEM")
                .unwrap_or_else(|e| panic!("failed to load KSEF_E2E_CERT_PEM: {e}"));
            let key_pem = read_pem_env("KSEF_E2E_KEY_PEM")
                .unwrap_or_else(|e| panic!("failed to load KSEF_E2E_KEY_PEM: {e}"));
            OpenSslXadesSigner::from_pem(key_pem, cert_pem)
        }
        (false, false) if e2e_environment() != KSeFEnvironment::Production => {
            eprintln!("KSEF_E2E_CERT_PEM/KEY_PEM not set — auto-generating self-signed for test");
            OpenSslXadesSigner::generate_self_signed_for_nip(&e2e_nip())
                .unwrap_or_else(|e| panic!("auto cert generation failed: {e}"))
        }
        (false, false) => panic!(
            "missing KSEF_E2E_CERT_PEM and KSEF_E2E_KEY_PEM (required for production E2E tests)"
        ),
        _ => panic!("KSEF_E2E_CERT_PEM and KSEF_E2E_KEY_PEM must be provided together"),
    }
}

mod fixtures {
    use ksef_core::domain::invoice::*;
    use ksef_core::domain::nip_account::NipAccountId;

    /// A test invoice using the E2E NIP from env (or default sandbox NIP).
    pub fn test_invoice() -> Invoice {
        let nip = super::e2e_nip();
        let issue_date = chrono::Utc::now().date_naive();

        Invoice {
            id: InvoiceId::new(),
            nip_account_id: NipAccountId::from_uuid(uuid::Uuid::from_u128(1)),
            direction: Direction::Outgoing,
            status: InvoiceStatus::Draft,
            invoice_type: InvoiceType::Vat,
            invoice_number: format!("E2E/2026/{}", uuid::Uuid::new_v4().as_simple()),
            issue_date,
            sale_date: Some(issue_date),
            corrected_invoice_number: None,
            correction_reason: None,
            original_ksef_number: None,
            advance_payment_date: None,
            seller: Party {
                nip: Some(nip.clone()),
                name: "E2E Test Seller".to_string(),
                address: Address {
                    country_code: CountryCode::pl(),
                    line1: "ul. Testowa 1".to_string(),
                    line2: "00-001 Warszawa".to_string(),
                },
            },
            buyer: Party {
                nip: Some(nip),
                name: "E2E Test Buyer".to_string(),
                address: Address {
                    country_code: CountryCode::pl(),
                    line1: "ul. Odbiorcza 5".to_string(),
                    line2: "00-002 Warszawa".to_string(),
                },
            },
            currency: Currency::pln(),
            line_items: vec![LineItem {
                line_number: 1,
                description: "E2E test service".to_string(),
                unit: Some("szt".to_string()),
                quantity: Quantity::integer(1),
                unit_net_price: Some(Money::from_pln(100, 0)),
                net_value: Money::from_pln(100, 0),
                vat_rate: VatRate::Rate23,
                vat_amount: Money::from_pln(23, 0),
                gross_value: Money::from_pln(123, 0),
            }],
            total_net: Money::from_pln(100, 0),
            total_vat: Money::from_pln(23, 0),
            total_gross: Money::from_pln(123, 0),
            payment_method: Some(PaymentMethod::Transfer),
            payment_deadline: Some(issue_date + chrono::Duration::days(14)),
            bank_account: Some("PL61109010140000071219812874".to_string()),
            ksef_number: None,
            ksef_error: None,
            raw_xml: None,
        }
    }
}

fn e2e_api() -> KSeFApiClient {
    KSeFApiClient::new(e2e_environment())
}

/// Step 1: Verify we can get a challenge from KSeF.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox"]
async fn auth_challenge_returns_valid_challenge() {
    let api = e2e_api();
    let nip = e2e_nip();

    let challenge = api.request_challenge(&nip).await.unwrap();

    println!("Challenge: {}", challenge.challenge);
    println!("Timestamp: {}", challenge.timestamp);

    assert!(!challenge.challenge.is_empty());
    assert!(challenge.challenge.contains("-CR-"));
    assert!(!challenge.timestamp.is_empty());
}

/// Step 2: Verify we can sign the challenge with XAdES.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + valid KSEF_E2E_CERT_PEM and KSEF_E2E_KEY_PEM"]
async fn xades_sign_and_submit_succeeds() {
    let api = e2e_api();
    let nip = e2e_nip();
    let signer = load_e2e_signer_from_env();

    // Get challenge
    let challenge = api.request_challenge(&nip).await.unwrap();
    println!("Got challenge: {}", challenge.challenge);

    // Sign it
    let signed = signer.sign_auth_request(&challenge, &nip).await.unwrap();
    println!("Signed XML size: {} bytes", signed.as_bytes().len());

    // Submit XAdES (fail-fast: any auth error fails the test).
    let auth_ref = api.authenticate_xades(&signed).await.unwrap();
    println!("Auth reference: {auth_ref}");
}

/// Step 3: Verify FA(3) XML generation produces valid XML.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox"]
async fn fa3_xml_generation_produces_valid_xml() {
    let invoice = fixtures::test_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();

    println!("Generated FA(3) XML ({} bytes):", xml.as_bytes().len());
    println!("{}", xml.as_str());

    assert!(
        xml.as_str()
            .contains("http://crd.gov.pl/wzor/2025/06/25/13775/")
    );
    assert!(xml.as_str().contains("<Faktura"));
    assert!(xml.as_str().contains("<FaWiersz>"));
}

/// Step 4: Verify encryption produces valid output.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + public key endpoint access"]
async fn public_key_fetch_and_invoice_encryption() {
    let api = e2e_api();

    // Fetch KSeF public keys
    let keys = api.fetch_public_keys().await.unwrap();
    println!("Got {} public keys", keys.len());
    for key in &keys {
        println!("  Key ID: {}", key.id());
        println!(
            "  PEM starts with: {}...",
            &key.pem()[..50.min(key.pem().len())]
        );
    }

    let key = keys.first().expect("KSeF returned empty public key list");
    let invoice = fixtures::test_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();
    let encryptor = AesCbcEncryptor;
    let encrypted = encryptor.encrypt(&xml, key).await.unwrap();
    println!(
        "Encrypted: aes_key={} bytes, iv={} bytes, data={} bytes",
        encrypted.aes_key().len(),
        encrypted.iv().len(),
        encrypted.data().len()
    );
}

/// Full flow: challenge → sign → auth → session → send → close.
/// This is the ultimate validation of our architecture.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + valid test credentials"]
async fn full_e2e_send_invoice() {
    let api = e2e_api();
    let nip = e2e_nip();
    let signer = load_e2e_signer_from_env();
    let encryptor = AesCbcEncryptor;

    println!("=== KSeF E2E Test ===");

    // 1. Challenge
    println!("\n--- Step 1: Request challenge ---");
    let challenge = api.request_challenge(&nip).await.unwrap();
    println!("Challenge: {}", challenge.challenge);

    // 2. Sign
    println!("\n--- Step 2: Sign with XAdES ---");
    let signed = signer.sign_auth_request(&challenge, &nip).await.unwrap();
    println!("Signed XML: {} bytes", signed.as_bytes().len());

    // 3. Authenticate
    println!("\n--- Step 3: Submit XAdES ---");
    let auth_ref = api.authenticate_xades(&signed).await.unwrap();
    println!("Auth reference: {auth_ref}");

    // 4. Poll status
    println!("\n--- Step 4: Poll auth status ---");
    let mut retries = 10;
    loop {
        let status = api.poll_auth_status(&auth_ref).await.unwrap();
        println!("Status: {status:?}");
        match status {
            AuthStatus::Completed => break,
            AuthStatus::Processing if retries > 0 => {
                retries -= 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            other => panic!("Unexpected auth status: {other:?}"),
        }
    }

    // 5. Redeem token
    println!("\n--- Step 5: Redeem JWT ---");
    let token_pair = api.redeem_token(&auth_ref).await.unwrap();
    println!(
        "Access token: {}...",
        &token_pair.access_token.as_str()[..20.min(token_pair.access_token.as_str().len())]
    );

    // 6. Generate + encrypt invoice
    println!("\n--- Step 6: Generate and encrypt invoice ---");
    let invoice = fixtures::test_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();
    println!("FA(3) XML: {} bytes", xml.as_bytes().len());

    let keys = api.fetch_public_keys().await.unwrap();
    let encrypted = encryptor.encrypt(&xml, &keys[0]).await.unwrap();
    println!("Encrypted: {} bytes", encrypted.data().len());

    // 7. Open session (requires encryption metadata from the invoice payload)
    println!("\n--- Step 7: Open session ---");
    let session = api
        .open_session(&token_pair.access_token, &encrypted)
        .await
        .unwrap();
    println!("Session: {session}");

    // 8. Send invoice
    println!("\n--- Step 8: Send invoice ---");
    let ksef_number = api
        .send_invoice(&token_pair.access_token, &session, &encrypted)
        .await
        .unwrap();
    println!("KSeF number: {ksef_number}");

    // 9. Close session
    println!("\n--- Step 9: Close session ---");
    let upo = api
        .close_session(&token_pair.access_token, &session)
        .await
        .unwrap();
    println!("UPO reference: {}", upo.reference);

    println!("\n=== E2E Test PASSED ===");
    println!(
        "Invoice {} sent to KSeF as {ksef_number}",
        invoice.invoice_number
    );
}

/// Helper: authenticate and return an access token for query/fetch tests.
async fn authenticate_for_query() -> (ksef_core::domain::auth::TokenPair, KSeFApiClient, Nip) {
    let api = e2e_api();
    let nip = e2e_nip();
    let signer = load_e2e_signer_from_env();

    let challenge = api.request_challenge(&nip).await.unwrap();
    let signed = signer.sign_auth_request(&challenge, &nip).await.unwrap();
    let auth_ref = api.authenticate_xades(&signed).await.unwrap();

    let mut retries = 10;
    loop {
        let status = api.poll_auth_status(&auth_ref).await.unwrap();
        match status {
            AuthStatus::Completed => break,
            AuthStatus::Processing if retries > 0 => {
                retries -= 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            other => panic!("Unexpected auth status: {other:?}"),
        }
    }

    let token_pair = api.redeem_token(&auth_ref).await.unwrap();
    (token_pair, api, nip)
}

/// Step 5: Query invoices from KSeF (POST /invoices/query/metadata).
///
/// Validates: correct HTTP method (POST), JSON body format, date range,
/// SubjectType PascalCase, response parsing including nested seller.nip.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + valid test credentials"]
async fn query_invoices_by_subject_type() {
    let (token_pair, api, _nip) = authenticate_for_query().await;

    println!("\n--- Query invoices (Subject1, last 30 days) ---");

    let today = chrono::Local::now().date_naive();
    let month_ago = today - chrono::Duration::days(30);

    let query = InvoiceQuery {
        date_from: month_ago,
        date_to: today,
        subject_type: SubjectType::Subject1,
    };

    let invoices = api
        .query_invoices(&token_pair.access_token, &query)
        .await
        .unwrap();

    println!("Found {} invoices (Subject1)", invoices.len());
    for inv in &invoices {
        println!(
            "  {} | {} | NIP: {}",
            inv.ksef_number, inv.invoice_date, inv.subject_nip
        );
    }

    // Also query Subject2 (buyer)
    println!("\n--- Query invoices (Subject2, last 30 days) ---");
    let query2 = InvoiceQuery {
        date_from: month_ago,
        date_to: today,
        subject_type: SubjectType::Subject2,
    };

    let invoices2 = api
        .query_invoices(&token_pair.access_token, &query2)
        .await
        .unwrap();

    println!("Found {} invoices (Subject2)", invoices2.len());
    for inv in &invoices2 {
        println!(
            "  {} | {} | NIP: {}",
            inv.ksef_number, inv.invoice_date, inv.subject_nip
        );
    }
}

/// Step 6: Fetch a single invoice XML from KSeF and parse it into domain Invoice.
///
/// Requires at least one invoice to exist (run full_e2e_send_invoice first).
/// Validates: GET /invoices/ksef/{number}, FA(3) XML parsing, domain mapping.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + valid test credentials + at least one sent invoice"]
async fn fetch_invoice_xml_and_parse_to_domain() {
    let (token_pair, api, _nip) = authenticate_for_query().await;

    // First query to find an invoice to fetch
    let today = chrono::Local::now().date_naive();
    let month_ago = today - chrono::Duration::days(30);

    let query = InvoiceQuery {
        date_from: month_ago,
        date_to: today,
        subject_type: SubjectType::Subject1,
    };

    let invoices = api
        .query_invoices(&token_pair.access_token, &query)
        .await
        .unwrap();

    assert!(
        !invoices.is_empty(),
        "no invoices found for fetch test; run full_e2e_send_invoice first"
    );

    let target = &invoices[0];
    println!("\n--- Fetch invoice {} ---", target.ksef_number);

    let untrusted_xml = api
        .fetch_invoice(&token_pair.access_token, &target.ksef_number)
        .await
        .unwrap();

    println!("Fetched XML: {} bytes", untrusted_xml.as_bytes().len());
    println!(
        "XML preview: {}...",
        &untrusted_xml.as_str()[..200.min(untrusted_xml.as_str().len())]
    );

    let xml = InvoiceXml::from_untrusted(untrusted_xml).unwrap();

    // Parse the real XML into domain Invoice
    let parsed = xml_to_invoice(&xml, Direction::Outgoing, &target.ksef_number).unwrap();

    println!("\n--- Parsed invoice ---");
    println!("  Number:    {}", parsed.invoice_number);
    println!(
        "  Seller:    {} (NIP: {})",
        parsed.seller.name,
        parsed.seller.nip.as_ref().map_or("-", |nip| nip.as_str())
    );
    println!(
        "  Buyer:     {} (NIP: {})",
        parsed.buyer.name,
        parsed.buyer.nip.as_ref().map_or("-", |nip| nip.as_str())
    );
    println!("  Net:       {}", parsed.total_net);
    println!("  VAT:       {}", parsed.total_vat);
    println!("  Gross:     {}", parsed.total_gross);
    println!("  Items:     {}", parsed.line_items.len());
    let payment_code = parsed
        .payment_method
        .as_ref()
        .map_or_else(|| "-".to_string(), |method| method.fa3_code().to_string());
    println!("  Payment:   {payment_code}");
    println!("  Status:    {}", parsed.status);
    println!("  Direction: {}", parsed.direction);
    println!("  KSeF #:    {}", parsed.ksef_number.as_ref().unwrap());

    // Verify key properties
    assert!(!parsed.invoice_number.is_empty());
    assert!(!parsed.seller.name.is_empty());
    assert!(!parsed.buyer.name.is_empty());
    assert_eq!(
        parsed.status,
        ksef_core::domain::invoice::InvoiceStatus::Fetched
    );
    assert_eq!(parsed.direction, Direction::Outgoing);
    assert!(parsed.raw_xml.is_some());
    assert!(parsed.line_items.len() >= 1);

    println!("\n=== Fetch + Parse E2E PASSED ===");
}

/// Step 7: Verify refresh token flow works.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + valid test credentials"]
async fn refresh_token_returns_new_valid_pair() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let refreshed = api.refresh_token(&token_pair.refresh_token).await.unwrap();

    assert!(!refreshed.access_token.as_str().is_empty());
    assert!(!refreshed.refresh_token.as_str().is_empty());
    assert!(refreshed.access_token_expires_at > chrono::Utc::now());
    assert!(refreshed.refresh_token_expires_at > chrono::Utc::now());
}

/// Step 8: Verify permissions endpoints end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + permissions management rights"]
async fn permissions_grant_query_revoke_roundtrip() {
    let (token_pair, api, nip) = authenticate_for_query().await;

    let query = PermissionQueryRequest {
        context_nip: nip.clone(),
        authorized_nip: Some(nip.clone()),
        permission: Some(PermissionType::InvoiceRead),
    };

    let before = api
        .query_permissions(&token_pair.access_token, &query)
        .await
        .unwrap();
    println!("Permissions before grant: {}", before.len());

    let grant = PermissionGrantRequest {
        context_nip: nip.clone(),
        authorized_nip: nip.clone(),
        permissions: vec![PermissionType::InvoiceRead],
    };
    api.grant_permissions(&token_pair.access_token, &grant)
        .await
        .unwrap();

    let after_grant = api
        .query_permissions(&token_pair.access_token, &query)
        .await
        .unwrap();
    println!("Permissions after grant: {}", after_grant.len());
    assert!(after_grant.len() >= before.len());

    let revoke = PermissionRevokeRequest {
        context_nip: nip.clone(),
        authorized_nip: nip,
        permissions: vec![PermissionType::InvoiceRead],
    };
    api.revoke_permissions(&token_pair.access_token, &revoke)
        .await
        .unwrap();

    let after_revoke = api
        .query_permissions(&token_pair.access_token, &query)
        .await
        .unwrap();
    println!("Permissions after revoke: {}", after_revoke.len());
}

/// Step 9: Verify token management endpoints end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + credentials management rights"]
async fn token_generate_query_get_revoke_roundtrip() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let generated = api
        .generate_token(
            &token_pair.access_token,
            &TokenGenerateRequest {
                permissions: vec![PermissionType::InvoiceRead],
                description: Some(format!("e2e-token-{}", uuid::Uuid::new_v4())),
                valid_to: Some(chrono::Utc::now() + chrono::Duration::days(7)),
            },
        )
        .await
        .unwrap();
    println!("Generated token id: {}", generated.id);
    assert!(!generated.id.is_empty());

    let fetched = api
        .get_token(&token_pair.access_token, &generated.id)
        .await
        .unwrap();
    assert_eq!(fetched.id, generated.id);

    let queried = api
        .query_tokens(
            &token_pair.access_token,
            &TokenQueryRequest {
                status: None,
                limit: Some(50),
                offset: Some(0),
            },
        )
        .await
        .unwrap();
    println!("Query token total: {}", queried.total);
    assert!(queried.items.iter().any(|item| item.id == generated.id));

    api.revoke_token(&token_pair.access_token, &generated.id)
        .await
        .unwrap();

    let revoked = api
        .get_token(&token_pair.access_token, &generated.id)
        .await
        .unwrap();
    assert!(matches!(
        revoked.status,
        ksef_core::domain::token_mgmt::TokenStatus::Revoked
            | ksef_core::domain::token_mgmt::TokenStatus::Expired
    ));
}

/// Step 10: Verify rate-limits endpoints end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox"]
async fn effective_rate_limits_match_context_and_subject_views() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let effective = api
        .get_effective_limits(&token_pair.access_token)
        .await
        .unwrap();
    println!(
        "Effective limits: contexts={}, subjects={}",
        effective.contexts.len(),
        effective.subjects.len()
    );

    let contexts = api
        .get_context_limits(&token_pair.access_token)
        .await
        .unwrap();
    let subjects = api
        .get_subject_limits(&token_pair.access_token)
        .await
        .unwrap();

    assert_eq!(effective.contexts, contexts);
    assert_eq!(effective.subjects, subjects);
}

/// Step 11: Verify PEPPOL providers query end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox"]
async fn peppol_providers_query_with_pagination() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let response = api
        .query_providers(
            &token_pair.access_token,
            &PeppolQueryRequest {
                page_offset: 0,
                page_size: 20,
            },
        )
        .await
        .unwrap();

    println!(
        "PEPPOL providers: total={}, page_items={}",
        response.total,
        response.items.len()
    );
    let page_items_u32 = u32::try_from(response.items.len()).unwrap();
    assert!(response.total >= page_items_u32);
}

/// Step 12: Verify certificates endpoints (limits + query) end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + certificate management rights"]
async fn certificate_limits_and_enrollment_query() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let limits = api.get_limits(&token_pair.access_token).await.unwrap();
    println!(
        "Certificate limits: max_active={}, active={}, pending={}",
        limits.max_active, limits.active, limits.pending
    );
    assert!(limits.active.saturating_add(limits.pending) <= limits.max_active);

    let pending = api
        .query_certificates(
            &token_pair.access_token,
            &CertificateQueryRequest {
                status: None,
                limit: Some(50),
                offset: Some(0),
            },
        )
        .await
        .unwrap();
    println!("Pending certificate enrollments: {}", pending.len());
}

/// Step 13: Verify asynchronous export flow end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + export rights"]
async fn export_start_and_poll_status() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let today = chrono::Local::now().date_naive();
    let week_ago = today - chrono::Duration::days(7);

    let created = api
        .start_export(
            &token_pair.access_token,
            &ExportRequest {
                query: InvoiceQuery {
                    date_from: week_ago,
                    date_to: today,
                    subject_type: SubjectType::Subject1,
                },
            },
        )
        .await
        .unwrap();

    println!(
        "Export job created: reference={}, status={:?}",
        created.reference_number, created.status
    );
    assert!(!created.reference_number.is_empty());

    let status = api
        .get_export_status(&token_pair.access_token, &created.reference_number)
        .await
        .unwrap();
    println!(
        "Export status: reference={}, status={:?}, download_url={:?}",
        status.reference_number, status.status, status.download_url
    );
    assert_eq!(status.reference_number, created.reference_number);
}

/// Step 14: Verify auth sessions listing end-to-end.
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox"]
async fn auth_sessions_list_includes_current() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let sessions = api.list_sessions(&token_pair.access_token).await.unwrap();
    println!("Auth sessions count: {}", sessions.len());
    assert!(!sessions.is_empty());
    assert!(
        sessions.iter().any(|s| s.current),
        "expected at least one current auth session"
    );
}

/// Step 15: Verify batch workflow end-to-end (open -> upload parts -> status -> close).
#[tokio::test]
#[ignore = "requires network access to KSeF sandbox + batch endpoint rights"]
async fn batch_open_upload_parts_and_close() {
    let (token_pair, api, _) = authenticate_for_query().await;

    let invoice = fixtures::test_invoice();
    let xml = invoice_to_xml(&invoice).unwrap();
    let batch_archive = BatchFileBuilder::default()
        .build(&[(
            format!("{}.xml", invoice.invoice_number),
            xml.as_bytes().to_vec(),
        )])
        .unwrap();

    let opened = api
        .open_batch_session(
            &token_pair.access_token,
            &BatchOpenRequest {
                file: batch_archive.file_info.clone(),
                parts: batch_archive.parts.clone(),
            },
        )
        .await
        .unwrap();
    println!(
        "Batch session opened: reference={}, status={:?}",
        opened.reference_number, opened.status
    );

    for part in &batch_archive.parts {
        let start = usize::try_from(part.offset_bytes).unwrap();
        let size = usize::try_from(part.size_bytes).unwrap();
        let payload = &batch_archive.zip_bytes[start..start + size];

        api.upload_part(
            &token_pair.access_token,
            &PartUploadRequest {
                session_reference: opened.reference_number.clone(),
                upload_url: None,
                part: part.clone(),
            },
            payload,
        )
        .await
        .unwrap();
    }

    let status = api
        .get_batch_status(&token_pair.access_token, &opened.reference_number)
        .await
        .unwrap();
    println!(
        "Batch status after uploads: reference={}, status={:?}",
        status.reference_number, status.status
    );

    let closed = api
        .close_batch_session(&token_pair.access_token, &opened.reference_number)
        .await
        .unwrap();
    println!(
        "Batch session closed: reference={}, status={:?}",
        closed.reference_number, closed.status
    );
    assert_eq!(closed.reference_number, opened.reference_number);
}
