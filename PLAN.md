# KSeF API v2 Full Coverage — Implementation Plan

## Overview

Bring the Rust `ksef-core` library and `ksef-server` dashboard to full KSeF API v2 feature parity. This adds 20 feature areas identified by comparing against the TypeScript reference client (`ksef-client-ts`), covering: rate limiting, batch sessions, offline invoices, permissions, token management, QR codes, invoice export, all invoice types, PEPPOL, certificate management, and more.

## Engineering Principles (Non-Negotiable)

These principles govern every line of code in this plan. They are constraints, not suggestions.

1. **Protocol first** — define traits, API contracts, and type signatures BEFORE implementation. The protocol drives the design. Every new feature starts with the port trait, not the HTTP client.
2. **Test-driven development** — write the failing test first, then implement. Tests ARE the spec. No implementation without a corresponding test. Contract test suites for port traits validate both mocks and real implementations against the same spec.
3. **Fail fast, no fallbacks** — errors surface immediately. No silent swallowing, no default fallbacks that mask bugs. `Result<T, E>` everywhere. If KSeF returns unexpected data → `Err`, never silently degrade. Missing required fields → `XmlError`, not a default value.
4. **Clean architecture** — domain logic is independent of frameworks, DB, HTTP. Dependencies point inward: `domain ← ports ← services ← infra`. No `reqwest` types in domain. No SQL in services.
5. **Idiomatic Rust** — newtype wrappers for domain types, `thiserror` for error hierarchies, ownership over cloning, builder patterns where construction is complex.
6. **Clean semantics** — names precisely reflect intent. No ambiguous abbreviations. Code reads like documentation.

### TDD Workflow (Every Task)

```
1. Write port trait (protocol first)
2. Write contract test suite against the trait
3. Write mock implementation — verify contract tests pass
4. Write real implementation — verify same contract tests pass
5. Write service tests using mock (tests ARE the spec)
6. Implement service — tests go green
7. Wire into server — integration test
```

## Goals

- **Full KSeF API v2 coverage** — every documented endpoint has a Rust port trait + HTTP implementation
- **Library-first** — `ksef-core` works standalone without Axum/DB; server is one consumer
- **Production resilience** — rate limiting, retry with backoff, structured errors
- **All invoice types** — full parsers for FA, Kor, Zal, Roz, Upr, PEF, RR and their variants
- **Dashboard parity** — SSR Askama UI for all new features

## Non-Goals

- SPA frontend rewrite (staying with SSR Askama)
- Multi-tenant support (single company per deployment)
- KSeF API v1 backwards compatibility
- Automatic background polling (infrastructure ready, not wired)

## Assumptions and Constraints

- Single `ksef-core` crate with modules (no workspace split)
- In-memory state for hot path (rate limiter), DB for persistent state (tokens, sessions)
- `zip` crate for batch ZIP archives
- `qrcode` + `image` crates for QR code generation (PNG/SVG)
- ECDSA P-256 as optional future-proofing, RSA remains primary
- Test data service behind `cfg(test)` — not compiled into release builds
- Testing: unit mocks + PG testcontainers + KSeF sandbox E2E suite
- Existing ~150 tests must remain green throughout
- KSeF test sandbox at `api-test.ksef.mf.gov.pl`

## Requirements

### Functional

- All 9 KSeF v2 service areas covered: auth, sessions, invoices, permissions, tokens, rate limits, PEPPOL, certificates, test data
- 12 invoice type variants parseable from XML
- Batch session workflow: create ZIP → open batch → multi-part upload → monitor → close
- Offline invoice workflow: generate → sign → QR codes (KOD I + KOD II) → submit when back online
- Permissions CRUD for all 7 grant types
- QR code generation as PNG/SVG with configurable parameters
- Invoice export with async polling

### Non-Functional

- Rate limiting: token bucket per-second/minute/hour with burst allowance
- Retry: exponential backoff with jitter, configurable max retries, respects Retry-After header
- Structured errors: KSeF error codes parsed from API responses, typed error variants — no stringly-typed errors hiding structure
- All new ports are `Send + Sync + 'static` (library constraint)
- No panics in library code — all failures via `Result`
- **Every port trait has a contract test suite** — the same test suite runs against mock AND real impl
- **No silent fallbacks** — if an API response is missing a field the spec says is required, that's an error, not a default
- **No `unwrap()` in non-test code** — `expect()` with message only where invariant is proven

## Technical Design

### New Dependencies

```toml
# Cargo.toml workspace additions
zip = "2"                    # Batch ZIP creation
qrcode = "0.14"             # QR code matrix generation
image = "0.25"              # QR PNG rendering
base64 = "0.22"             # Explicit base64 (replacing openssl::base64)
sha2 = "0.10"               # SHA-256 hashing (pure Rust)
p256 = "0.13"               # ECDSA P-256 (optional, feature-gated)
```

### Architecture: New Modules

```
ksef-core/src/
├── domain/
│   ├── invoice.rs           # + InvoiceType enum, correction fields
│   ├── batch.rs             # NEW: BatchFileInfo, PartUploadRequest, BatchSession
│   ├── offline.rs           # NEW: OfflineMode, OfflineInvoice, OfflineCertificate
│   ├── permission.rs        # NEW: PermissionType variants, grant/revoke requests
│   ├── token_mgmt.rs        # NEW: ManagedToken, TokenStatus, TokenPermission
│   ├── rate_limit.rs        # NEW: RateLimitConfig, RateLimitStatus, RateLimitCategory
│   ├── certificate_mgmt.rs  # NEW: CertificateEnrollment, CertificateLimits
│   ├── peppol.rs            # NEW: PeppolProvider
│   ├── qr.rs                # NEW: QRCodeData, QRCodeFormat, KodI, KodII
│   ├── identifiers.rs       # NEW: Pesel, NipVatUe, PeppolId, InternalId, Fingerprint
│   └── ...existing...
├── ports/
│   ├── ksef_client.rs       # + session status/list, batch, export, UPO per invoice
│   ├── ksef_auth.rs         # + token-based auth, auth session management
│   ├── ksef_permissions.rs  # NEW: full permissions port
│   ├── ksef_tokens.rs       # NEW: token management port
│   ├── ksef_certificates.rs # NEW: certificate management port
│   ├── ksef_peppol.rs       # NEW: PEPPOL port
│   ├── ksef_rate_limits.rs  # NEW: rate limit query port
│   └── ...existing...
├── services/
│   ├── batch_service.rs     # NEW: batch session orchestration
│   ├── offline_service.rs   # NEW: offline invoice workflow
│   ├── permission_service.rs# NEW: permission orchestration
│   ├── token_mgmt_service.rs# NEW: token CRUD orchestration
│   ├── export_service.rs    # NEW: invoice export with polling
│   ├── qr_service.rs        # NEW: QR code generation
│   └── ...existing...
├── infra/
│   ├── http/
│   │   ├── rate_limiter.rs  # NEW: token bucket rate limiter
│   │   └── retry.rs         # NEW: exponential backoff with jitter
│   ├── ksef/
│   │   ├── permissions_client.rs  # NEW
│   │   ├── tokens_client.rs       # NEW
│   │   ├── certificates_client.rs # NEW
│   │   ├── peppol_client.rs       # NEW
│   │   ├── batch_client.rs        # NEW
│   │   ├── rate_limits_client.rs  # NEW
│   │   └── ...existing (extended)...
│   ├── fa3/
│   │   ├── parse.rs         # + dispatch by InvoiceType
│   │   ├── parse_kor.rs     # NEW: correction invoice parser
│   │   ├── parse_zal.rs     # NEW: advance payment parser
│   │   ├── parse_roz.rs     # NEW: split invoice parser
│   │   ├── parse_upr.rs     # NEW: simplified invoice parser
│   │   ├── parse_pef.rs     # NEW: proforma parser
│   │   ├── parse_rr.rs      # NEW: RR invoice parser
│   │   └── serialize.rs     # + serializers for new types
│   ├── qr/
│   │   └── generator.rs     # NEW: QR PNG/SVG generation
│   ├── batch/
│   │   └── zip_builder.rs   # NEW: ZIP archive creation
│   └── validation/
│       └── mod.rs            # NEW: PESEL, NIP-VAT-UE, PEPPOL ID, etc.
└── error.rs                  # + KSeFApiError with structured codes
```

### Structured Error Response

```rust
/// Parsed KSeF API error response
#[derive(Debug)]
pub struct KSeFApiErrorDetail {
    pub status_code: u16,
    pub ksef_code: Option<String>,        // e.g., "AUTH-001"
    pub description: String,
    pub details: Vec<String>,
    pub reference_number: Option<String>,
    pub processing_code: Option<u32>,
}

// New KSeFError variants:
pub enum KSeFError {
    // ...existing...
    #[error("KSeF API error: {0}")]
    ApiError(KSeFApiErrorDetail),
    
    #[error("rate limit exceeded, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
}
```

### Rate Limiter Design

```rust
pub struct TokenBucketRateLimiter {
    buckets: Arc<Mutex<HashMap<RateLimitCategory, TokenBucket>>>,
    config: RateLimitConfig,
}

struct TokenBucket {
    tokens_per_second: f64,
    tokens_per_minute: f64,
    tokens_per_hour: f64,
    // Sliding window counters
    second_window: VecDeque<Instant>,
    minute_window: VecDeque<Instant>,
    hour_window: VecDeque<Instant>,
}

impl TokenBucketRateLimiter {
    pub async fn acquire(&self, category: RateLimitCategory) -> Result<(), KSeFError>;
    pub fn status(&self, category: RateLimitCategory) -> RateLimitStatus;
    pub fn update_limits(&self, limits: EffectiveApiRateLimits);
}
```

### Retry Design

```rust
pub struct RetryPolicy {
    pub max_retries: u32,          // default: 3
    pub initial_delay_ms: u64,     // default: 1000
    pub max_delay_ms: u64,         // default: 30000
    pub multiplier: f64,           // default: 2.0
    pub jitter_factor: f64,        // default: 0.25
}

impl RetryPolicy {
    pub async fn execute<F, Fut, T, E>(&self, f: F) -> Result<T, E>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: RetryableError;
}

pub trait RetryableError {
    fn is_retryable(&self) -> bool;
    fn retry_after_ms(&self) -> Option<u64>;
}
```

---

## Implementation Plan

### Serial Dependencies (Must Complete First)

#### Phase 0: HTTP Infrastructure Foundation
**Prerequisite for:** All subsequent phases — every KSeF HTTP call flows through this layer
**Approach:** Protocol first → TDD → implement

| Task | Description | Output |
|------|-------------|--------|
| 0.1 | **Structured KSeF error parsing** — TDD: write tests first for parsing `{status: {code, description, details}}` JSON into `KSeFApiErrorDetail`. Test cases: valid error, missing fields → `Err`, 429 with Retry-After. Then implement `parse_ksef_error_response()`. Add `KSeFError::ApiError` and `KSeFError::RateLimited` variants. No fallback parsing — malformed error response is itself an error | Tests first → `error.rs` + parser fn |
| 0.2 | **`RetryableError` trait (protocol first)** — define trait: `is_retryable() -> bool`, `retry_after_ms() -> Option<u64>`. Implement for `KSeFError`: 5xx/429 retryable, 4xx not. Write contract tests. Then **`RetryPolicy` (TDD)** — write tests: succeeds on first try, retries on retryable error, gives up after max_retries, respects retry_after_ms, jitter within bounds. Then implement `infra/http/retry.rs` | Trait + contract tests → `RetryPolicy` impl + unit tests |
| 0.3 | **Rate limiter (TDD)** — write tests first: acquire succeeds under limit, acquire returns `RateLimited` when exceeded, window resets after time, per-category isolation, burst allowance. Then implement `infra/http/rate_limiter.rs` with `TokenBucketRateLimiter`. `Arc<Mutex<>>` in-memory state | Tests first → `TokenBucketRateLimiter` + `RateLimitConfig` + `RateLimitStatus` |
| 0.4 | **Integrate rate limiter + retry into HTTP clients** — inject via constructor: `new(client, base_url, rate_limiter, retry_policy)`. Wrap all `reqwest` calls. Update existing tests to pass rate limiter + retry | Updated constructors + all HTTP calls wrapped |
| 0.5 | **Verify existing tests pass** — `cargo test --workspace`, `cargo clippy --workspace` | All green, no regressions |

#### Phase 1: Domain Model Extensions
**Prerequisite for:** Workstreams A-F — new types used across all features
**Dependencies:** Phase 0
**Approach:** Domain types are pure — no dependencies on infra. Tests first for all validation logic. Newtype wrappers enforce invariants at construction time (fail fast).

| Task | Description | Output |
|------|-------------|--------|
| 1.1 | **Identifier types (TDD)** — tests first: valid/invalid PESEL (11 digits, checksum), NipVatUe (country prefix validation), PeppolId format, InternalId, Fingerprint. Then create `domain/identifiers.rs` with newtype wrappers. `FromStr` returns `DomainError` — no fallback parsing, invalid input is always an error | Tests first → newtype wrappers + `DomainError` variants |
| 1.2 | **Validation module (TDD)** — tests first for each validator. Then create `infra/validation/mod.rs`: `validate_email()`, `validate_phone()`, `validate_iso_country_code()`, `validate_file_size()`, `validate_date_range()`. Each returns `Result<(), DomainError>` — no boolean validators that lose error context | Tests first → validation fns |
| 1.3 | **InvoiceType enum (TDD)** — tests first: each variant roundtrips through `Display`/`FromStr`, `form_code()` returns correct `FormCode`, unknown string → `Err`. Then implement in `domain/invoice.rs`. No `Default` impl — invoice type must be explicitly chosen | Tests first → enum + FormCode |
| 1.4 | **Invoice domain extensions** — add fields: `invoice_type: InvoiceType`, `corrected_invoice_number: Option<String>`, `correction_reason: Option<String>`, `original_ksef_number: Option<KSeFNumber>`, `advance_payment_date: Option<NaiveDate>`. Update existing tests | Updated Invoice struct |
| 1.5 | **Permission domain types (TDD)** — tests for enum roundtrips and request validation. Then create `domain/permission.rs`: all 7 permission type enums + request/response structs. Enums are exhaustive — no catch-all variant that hides new API additions | Tests first → domain types |
| 1.6 | **Token management domain (TDD)** — tests for `TokenStatus` state machine transitions (fail fast on invalid transitions). Then create `domain/token_mgmt.rs` | Tests first → domain types |
| 1.7 | **Batch domain types** — create `domain/batch.rs`: `BatchFileInfo`, `BatchFilePartInfo`, `PartUploadRequest`, `BatchSessionStatus` state machine. Test status transitions | Domain types + tests |
| 1.8 | **Offline domain types (TDD)** — tests for `OfflineMode` deadline calculation, `OfflineInvoiceStatus` state machine. Then create `domain/offline.rs`. Deadline is an invariant, not a suggestion — expired = error | Tests first → domain types |
| 1.9 | **QR domain types** — create `domain/qr.rs`: `QRCodeData`, `QRCodeFormat` (Png/Svg), `QRCodeOptions`, `KodI`, `KodII`. URL format validation in tests | Domain types + tests |
| 1.10 | **Rate limit domain** — create `domain/rate_limit.rs`: `RateLimitCategory`, `EffectiveApiRateLimits`, `ContextLimits`, `SubjectLimits` | Domain types |
| 1.11 | **Certificate management domain** — create `domain/certificate_mgmt.rs`: `CertificateEnrollment`, `CertificateLimits`, `EnrollmentStatus`, `KsefCertificateType` | Domain types |
| 1.12 | **PEPPOL domain** — create `domain/peppol.rs`: `PeppolProvider` | Domain types |
| 1.13 | **UPO extensions** — extend `domain/session.rs`: `UpoVersion` enum (V4_2, V4_3), `UpoPageResponse` with download URL + expiry, `UpoDownloadResult` with optional hash | Updated types + tests |
| 1.14 | **Migration 003** — add `invoice_type VARCHAR`, `corrected_invoice_number VARCHAR`, `correction_reason TEXT`, `original_ksef_number VARCHAR`, `advance_payment_date DATE` to invoices table. Update status CHECK for new statuses | Migration file |
| 1.15 | **Update PG queries** — include new invoice columns in INSERT/SELECT/UPDATE. PG integration tests for new columns | Updated SQL + tests |
| 1.16 | **All tests green** — `cargo test --workspace`, `cargo clippy --workspace` | All green |

---

### Parallel Workstreams

After Phase 0 + Phase 1, these workstreams can execute independently.

#### Workstream A: Authentication & Security Extensions
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** B, C, D, E, F
**Approach:** Protocol first — define traits before any HTTP code. Contract test suites for each trait.

| Task | Description | Output |
|------|-------------|--------|
| A.1 | **Token-based auth port (protocol first)** — extend `KSeFAuth` trait: `authenticate_token(context: &ContextIdentifier, token: &str) -> Result<AuthReference, KSeFError>`. Write contract test: authenticate with valid token → AuthReference, invalid token → `Err` | Updated port + contract tests |
| A.2 | **Token-based auth HTTP impl (TDD)** — write test: mock HTTP returns auth reference after token encryption. Then implement in `HttpKSeFAuth`: encrypt token with RSA-OAEP + timestamp, POST `/auth/init-token-authentication`. Verify contract tests pass for real impl | Implementation passing contract tests |
| A.3 | **Auth session management port (protocol first)** — new `KSeFAuthSessions` trait: `list_sessions(token) -> Vec<AuthSessionInfo>`, `revoke_session(token, ref_num) -> Result<(), KSeFError>`, `revoke_current_session(token)`. Contract test suite | New port + contract tests |
| A.4 | **Auth session HTTP impl (TDD)** — tests first for GET/DELETE `/auth/sessions`. Then implement. Verify contract tests pass | Implementation + tests |
| A.5 | **ECDSA P-256 signing (TDD)** — tests first: detect key type (RSA vs EC), sign with ECDSA-SHA256 + IEEE P1363, verify signature roundtrip. Behind feature flag `ecdsa`. Then extend `XadesSigner`. Unknown key type → `CryptoError`, not a fallback to RSA | Tests first → updated signer + feature flag |
| A.6 | **Certificate management port (protocol first)** — new `KSeFCertificates` trait: `get_limits()`, `submit_enrollment()`, `get_enrollment_status()`, `query_certificates()`, `retrieve_certificates()`, `revoke_certificate()`. Contract test suite | New port + contract tests |
| A.7 | **Certificate management HTTP impl (TDD)** — tests first, then all `/certificates/` endpoints. Contract tests pass | Implementation passing contract tests |
| A.8 | **Security service: public key caching (TDD)** — test: first call fetches, second call returns cached, TTL expiry triggers re-fetch. Then implement in-memory cache with `Arc<Mutex<>>` + `Instant` TTL | Tests first → cached service |
| A.9 | **Update SessionService (TDD)** — test: `AuthMethod::Token` path works, `AuthMethod::XAdES` path unchanged. Then implement. Config enum selects method — no runtime sniffing, explicit choice | Tests first → updated service |

#### Workstream B: Session & Invoice Lifecycle
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** A, C, D, E, F
**Approach:** Protocol first for all new port methods. TDD for each HTTP implementation. No optional fields pretending to be required.

| Task | Description | Output |
|------|-------------|--------|
| B.1 | **Session status port (protocol first)** — extend `KSeFClient`: `get_session_status(token, session) -> Result<SessionStatusResponse, KSeFError>` with invoice counts (accepted/rejected), expiry time. Contract tests | Updated port + contract tests |
| B.2 | **Session listing port (protocol first)** — extend `KSeFClient`: `list_sessions(token, filters) -> Result<SessionsQueryResponse, KSeFError>` with pagination. Contract tests | Updated port + contract tests |
| B.3 | **Session invoice listing (protocol first)** — extend `KSeFClient`: `list_session_invoices(token, session)`, `list_failed_session_invoices(token, session)`. Contract tests | Updated port + contract tests |
| B.4 | **Session HTTP implementations (TDD)** — tests first for GET `/sessions/{ref}`, GET `/sessions`, GET `/sessions/{ref}/invoices`, GET `/sessions/{ref}/failed-invoices`. Missing response fields → `KSeFError`, not defaults. Then implement | Tests first → impl passing contract tests |
| B.5 | **Batch session port (protocol first)** — new trait methods or separate `KSeFBatchClient` trait: `open_batch_session()`, `close_batch_session()`, `upload_batch_part()`. Contract test suite | Port + contract tests |
| B.6 | **Batch ZIP builder (TDD)** — tests first: empty vec → error (not empty ZIP), single invoice → valid ZIP, multi-invoice → valid ZIP, split at 5MB boundary, SHA-256 per part correct. Then create `infra/batch/zip_builder.rs` | Tests first → `BatchFileBuilder` |
| B.7 | **Batch session HTTP impl (TDD)** — tests first for POST `/sessions/batch`, PUT `/sessions/batch/{ref}`, PUT to presigned URLs. Then implement. Contract tests pass | Tests first → impl |
| B.8 | **Batch service (TDD)** — tests first with mocks: happy path (encrypt → ZIP → open → upload → close), partial upload failure → error (no partial success hiding), rate limiter respected. Then implement `services/batch_service.rs` | Tests first → `BatchService` |
| B.9 | **Invoice export port (protocol first)** — `export_invoices()`, `get_export_status()`. Contract tests | Port + contract tests |
| B.10 | **Invoice export HTTP impl (TDD)** — tests first, then POST `/invoices/export`, GET `/invoices/export/{ref}` | Tests first → impl |
| B.11 | **Export service (TDD)** — tests: submit → poll pending → poll complete → return URL, poll → failed → error (no retry hiding failure). Then implement | Tests first → `ExportService` |
| B.12 | **UPO per invoice port (protocol first)** — `get_invoice_upo_by_ksef()`, `get_invoice_upo_by_reference()` with hash verification. If hash mismatch → `KSeFError`, not silent accept. Contract tests | Port + contract tests |
| B.13 | **UPO per invoice HTTP impl (TDD)** — tests first for GET `/invoices/ksef/{num}/upo`, GET `/invoices/{ref}/upo`. Then implement | Tests first → impl |
| B.14 | **Richer query filters (TDD)** — tests first for new filter fields: `ksef_reference_number`, `invoice_number_range`, `amount_filter`, `invoice_type`, `invoicing_mode`, `currency`, `continuation_token`, `sort_order`. Invalid combination → `DomainError`, not silent ignore | Tests first → updated domain + port + impl |
| B.15 | **Pagination with continuation tokens (TDD)** — tests: single page returns None token, multi-page auto-follows, truncated response flagged. Then update `query_invoices` response to `QueryResult<InvoiceMetadata>` | Tests first → updated types + impl |

#### Workstream C: Permissions & Token Management
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** A, B, D, E, F
**Approach:** Protocol first — full trait definitions before any HTTP code. All 7 permission types share pattern but each gets explicit trait method (no stringly-typed dispatch).

| Task | Description | Output |
|------|-------------|--------|
| C.1 | **Permissions port (protocol first)** — create `ports/ksef_permissions.rs` with `KSeFPermissions` trait: all grant/revoke/query methods with typed request/response structs (no `serde_json::Value` passthrough). Contract test suite: grant → query (visible) → revoke → query (gone) for each type | Full trait + contract tests |
| C.2 | **Permissions HTTP impl (TDD)** — tests first for each endpoint. Then create `infra/ksef/permissions_client.rs`. Verify contract tests pass for real impl. Each grant type has its own request body — no "generic grant" that loses type safety | Tests first → impl passing contract tests |
| C.3 | **Permissions service (TDD)** — tests first with mocks: validates inputs (invalid NIP → `DomainError`), delegates to port, surfaces errors. Then create `services/permission_service.rs`. Thin layer — no business logic duplication with domain | Tests first → `PermissionService` |
| C.4 | **Permissions edge case tests** — grant duplicate → handle idempotency or error, revoke non-existent → specific error, query empty → empty vec (not error), invalid permission type → compile-time error (exhaustive enum) | Edge case tests |
| C.5 | **Token management port (protocol first)** — create `ports/ksef_tokens.rs` with `KSeFTokens` trait: `generate_token()`, `query_tokens()`, `get_token()`, `revoke_token()`. Contract test suite: generate → query (visible) → revoke → query (gone) | Full trait + contract tests |
| C.6 | **Token management HTTP impl (TDD)** — tests first. Then create `infra/ksef/tokens_client.rs`. Contract tests pass | Tests first → impl |
| C.7 | **Token management service (TDD)** — tests first: generate with permissions, query with pagination, revoke active token, revoke already-revoked → error. Then create `services/token_mgmt_service.rs` | Tests first → `TokenMgmtService` |

#### Workstream D: Offline Invoices & QR Codes
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** A, B, C, E, F
**Approach:** TDD. QR URL format is protocol — test the exact URL spec first. Offline deadlines are invariants, not suggestions.

| Task | Description | Output |
|------|-------------|--------|
| D.1 | **QR code generator (TDD)** — tests first: KOD I URL format matches spec `https://qr-{env}.ksef.mf.gov.pl/invoice/{nip}/{date-DD-MM-YYYY}/{sha256base64url}`, generated PNG is decodable, SVG is valid XML. Then create `infra/qr/generator.rs`. Invalid input → error, not malformed QR | Tests first → `QRCodeGenerator` |
| D.2 | **QR service (TDD)** — tests first: invoice without XML → error (no empty QR), hash computation correct, date format DD-MM-YYYY verified. Then create `services/qr_service.rs` | Tests first → `QRService` |
| D.3 | **Offline invoice manager (TDD)** — tests first: generate with valid mode + cert → OfflineInvoice with QR codes, generate without cert for KOD II → error, mode determines deadline. Then create `services/offline_service.rs` | Tests first → `OfflineService` |
| D.4 | **Offline submission deadlines (TDD)** — tests first: `Offline24` = exactly 24h, `Offline` = configurable (default 48h), `Awaryjny` deadline from MF config, past deadline → `OfflineInvoiceStatus::Expired`. Deadline is checked at submission time — expired invoice cannot be submitted, period | Tests first → deadline logic |
| D.5 | **Offline invoice status tracking (TDD)** — tests first for state machine: Generated → Queued → Submitted → Accepted/Rejected. Generated → Expired (deadline passed). Invalid transition → `DomainError`. No implicit state changes | Tests first → state machine |
| D.6 | **Offline → online submission (TDD)** — tests first: submit batch of offline invoices, one expired → error for that one (batch continues), submission uses normal session flow. Then integrate with SessionService | Tests first → integration |
| D.7 | **Offline certificate handling (TDD)** — tests first: load PEM, load P12/PFX with password, invalid cert → `CryptoError`, extract signing key, extract serial number for KOD II | Tests first → certificate loader |

#### Workstream E: Invoice Type Parsers
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** A, B, C, D, F

| Task | Description | Output |
|------|-------------|--------|
| E.1 | **Parser dispatch** — update `fa3/parse.rs`: detect invoice type from XML root/header, dispatch to specific parser. Shared utility functions extracted from current FA(3) parser | Dispatcher + shared utils |
| E.2 | **Correction invoice parser (Kor)** — create `fa3/parse_kor.rs`: parses `<FakturaKorygujaca>` or correction-marked FA. Extracts `corrected_invoice_number`, `correction_reason`, `original_ksef_number`, before/after line items | Parser + tests with fixtures |
| E.3 | **Advance payment parser (Zal)** — create `fa3/parse_zal.rs`: parses advance payment invoices. Extracts advance amount, payment date, order reference | Parser + tests with fixtures |
| E.4 | **Split invoice parser (Roz)** — create `fa3/parse_roz.rs`: parses split payment invoices. Handles split payment flag, bank account for VAT split | Parser + tests |
| E.5 | **Simplified invoice parser (Upr)** — create `fa3/parse_upr.rs`: parses simplified invoices (no buyer NIP required, amount limits) | Parser + tests |
| E.6 | **Proforma parser (PEF)** — create `fa3/parse_pef.rs`: parses VatPef, VatPefSp, KorPef variants | Parser + tests |
| E.7 | **RR invoice parser** — create `fa3/parse_rr.rs`: parses VatRr, KorVatRr (farmer invoices — different schema structure) | Parser + tests |
| E.8 | **Combined variant parsers** — KorZal (correction of advance) and KorRoz (correction of split): compose correction + base type parsing logic | Parsers + tests |
| E.9 | **Serializers for new types** — extend `fa3/serialize.rs`: `invoice_to_xml()` handles all invoice types, generating correct XML structure per type | Serializers + round-trip tests |
| E.10 | **Round-trip tests** — for each invoice type: `create → serialize → parse → verify equivalence` | Integration tests |

#### Workstream F: Ancillary Services
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** A, B, C, D, E

| Task | Description | Output |
|------|-------------|--------|
| F.1 | **PEPPOL port** — create `ports/ksef_peppol.rs`: `KSeFPeppol` trait with `query_providers(page_offset, page_size) -> PeppolProvidersResponse` | Port |
| F.2 | **PEPPOL HTTP impl** — GET `/peppol/query` with pagination | Implementation + tests |
| F.3 | **Rate limit query port** — create `ports/ksef_rate_limits.rs`: `KSeFRateLimits` trait with `get_effective_limits()`, `get_context_limits()`, `get_subject_limits()` | Port |
| F.4 | **Rate limit query HTTP impl** — GET `/rate-limits`, GET `/limits/context`, GET `/limits/subject`. Feed results into `TokenBucketRateLimiter.update_limits()` for dynamic adjustment | Implementation + tests |
| F.5 | **Test data service** — behind `#[cfg(test)]` in `infra/ksef/test_data_client.rs`: `grant_permissions()`, `create_subject()`, `create_person()`, `set_session_limits()`, `set_rate_limits()`, etc. Only compiles in test builds | Test utility + tests |
| F.6 | **Sync rate limits on startup** — on `HttpKSeFClient` init: optionally fetch effective limits and configure rate limiter dynamically | Startup hook |

---

### Merge Phase

After all parallel workstreams complete.

#### Phase 2: Service Integration
**Dependencies:** Workstreams A-F

| Task | Description | Output |
|------|-------------|--------|
| 2.1 | **Wire new services in `main.rs`** — inject all new services: BatchService, OfflineService, PermissionService, TokenMgmtService, ExportService, QRService. All receive shared rate limiter + retry policy | Updated DI |
| 2.2 | **Extend AppState** — add all new services as `Arc<dyn Trait>` fields | Updated state |
| 2.3 | **New dashboard routes** — register routes for: batch management, permissions, tokens, export, offline invoices, QR preview, session monitoring | Route registration |
| 2.4 | **Dashboard: Batch management page** — form to upload invoices for batch, progress display, status monitoring | Askama templates |
| 2.5 | **Dashboard: Permissions page** — grant/revoke forms for all 7 types, query results table with pagination | Askama templates |
| 2.6 | **Dashboard: Token management page** — generate, list, revoke tokens | Askama templates |
| 2.7 | **Dashboard: Export page** — trigger export, poll status, download link | Askama templates |
| 2.8 | **Dashboard: Session monitor** — list active sessions, view status (invoice counts), close sessions | Askama templates |
| 2.9 | **Dashboard: Invoice detail enhancements** — show invoice type badge, QR code preview (KOD I), correction chain link, UPO download per invoice | Updated templates |
| 2.10 | **Dashboard: Offline invoices page** — generate offline invoice, show QR codes, track status, submit when online | Askama templates |
| 2.11 | **Update existing invoice list** — filter by invoice type, show type column, support new query filters in UI | Updated templates |

#### Phase 3: Integration Testing
**Dependencies:** Phase 2

| Task | Description | Output |
|------|-------------|--------|
| 3.1 | **PG migration test** — verify migration 003 runs on existing schema | Test |
| 3.2 | **New column round-trips** — invoice_type, correction fields persist correctly through PG | Tests |
| 3.3 | **Rate limiter integration test** — verify rate limiter blocks when limits exceeded, releases after window | Test |
| 3.4 | **Retry integration test** — mock server returns 503 twice then 200, verify retry succeeds | Test |
| 3.5 | **Batch ZIP round-trip** — create ZIP, verify structure, extract and verify contents | Test |
| 3.6 | **QR code round-trip** — generate KOD I, decode QR, verify URL format | Test |
| 3.7 | **All tests green** — `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` | All green |

#### Phase 4: KSeF Sandbox E2E Suite
**Dependencies:** Phase 3

| Task | Description | Output |
|------|-------------|--------|
| 4.1 | **E2E test infrastructure** — create `tests/e2e/` with shared setup: env config, NIP, auth helper. Requires `KSEF_E2E=1` env var to run (skipped by default) | Test harness |
| 4.2 | **E2E: Token-based auth** — authenticate with token, verify access token received | E2E test |
| 4.3 | **E2E: XAdES auth** — authenticate with certificate, full flow through redeem | E2E test |
| 4.4 | **E2E: Online session lifecycle** — open → send invoice → check status → close → get UPO | E2E test |
| 4.5 | **E2E: Invoice query with filters** — query by date range, type, amount; verify pagination | E2E test |
| 4.6 | **E2E: Invoice export** — trigger export, poll until complete | E2E test |
| 4.7 | **E2E: UPO per invoice** — download UPO by KSeF number, verify content | E2E test |
| 4.8 | **E2E: Permissions** — grant person permission, query, revoke, verify removed | E2E test |
| 4.9 | **E2E: Token management** — generate token, query, revoke | E2E test |
| 4.10 | **E2E: Batch session** — create batch ZIP (3 invoices), upload, monitor, close | E2E test |
| 4.11 | **E2E: Rate limit query** — fetch effective limits, verify response structure | E2E test |
| 4.12 | **E2E: Session listing** — list sessions, verify current session appears | E2E test |
| 4.13 | **E2E: Dashboard smoke test** — start server, navigate all new pages, verify renders | Manual verification checklist |

---

## Testing and Validation

### Test Pyramid

```
              ╱╲
             ╱  ╲           E2E KSeF sandbox: ~12 tests (4.2-4.12)
            ╱────╲
           ╱      ╲         PG integration: ~10 tests (3.1-3.6)
          ╱────────╲
         ╱          ╲       Integration: rate limiter, retry, ZIP, QR
        ╱────────────╲
       ╱              ╲     Service unit tests: ~40 tests (mock-based)
      ╱────────────────╲
     ╱                  ╲   XML parsers: ~60 tests (12 types × ~5 each)
    ╱────────────────────╲
   ╱                      ╲ Domain validation: ~30 tests (identifiers, types)
  ╱────────────────────────╲
```

**Estimated new test count: ~150+ tests**

### Key Test Scenarios

- **Rate limiter**: acquire 10 tokens in 1s with limit=5, verify 6th blocks
- **Retry**: mock 503→503→200, verify 2 retries + backoff delays
- **Batch ZIP**: 3 invoices → ZIP → extract → verify each file intact
- **QR KOD I**: generate URL, verify format matches spec
- **QR KOD II**: generate with cert, verify signature in URL
- **Correction parser**: parse Kor XML, verify corrected_invoice_number populated
- **Permission grant/revoke**: mock grant → query (visible) → revoke → query (gone)
- **Offline deadline**: generate Offline24, advance clock 25h, verify Expired status
- **Continuation token**: mock paginated response (3 pages), verify all items collected

---

## Verification Checklist

- [ ] `cargo check --workspace` — zero errors
- [ ] `cargo clippy --workspace -- -D warnings` — zero warnings
- [ ] `cargo test --workspace` — all tests pass (existing ~150 + new ~150)
- [ ] `cargo test --workspace -- --ignored` (with `KSEF_E2E=1`) — E2E tests pass against sandbox
- [ ] Rate limiter: `cargo test rate_limit` — token bucket works correctly
- [ ] Retry: `cargo test retry` — exponential backoff with jitter verified
- [ ] All 12 invoice type parsers have round-trip tests
- [ ] QR code: generated PNG/SVG files are valid (decodeable)
- [ ] Dashboard: every new page renders without error
- [ ] `cargo doc --workspace --no-deps` — all public types documented

---

## Rollout and Migration

### Database Migration

1. Run migration 003 (additive — new columns, no destructive changes)
2. Existing invoices get `invoice_type = 'Vat'` as default
3. No data migration needed — new columns are all `Option`

### Feature Rollout Order

1. **Phase 0** — deploy with rate limiting + retry (invisible to users, improves reliability)
2. **Workstreams A+B** — deploy auth improvements + session monitoring (dashboard enhancements)
3. **Workstream E** — deploy invoice type parsers (existing fetch flow handles new types)
4. **Workstreams C+F** — deploy permissions + tokens + PEPPOL (new dashboard pages)
5. **Workstream D** — deploy offline + QR (requires offline certificates configured)

### Rollback Plan

- All migrations are additive (new columns, no drops) — rollback = deploy previous binary
- Rate limiter can be disabled via config: `rate_limit_enabled: false`
- New services behind feature flags in server config if needed

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| KSeF sandbox instability blocks E2E tests | High | Medium | E2E tests gated behind `KSEF_E2E=1` env var, don't block CI. Mock-based tests are primary |
| Invoice type XML schemas differ from expectations | Medium | High | Collect real XML fixtures from sandbox for each type. Parser strict — fails fast on unknowns |
| Rate limiter overhead on hot path | Low | Medium | Benchmark. Sliding window with `VecDeque` is O(1) amortized. Switch to atomic counters if needed |
| Batch upload presigned URLs expire during slow upload | Medium | Medium | Track URL expiry, re-request if needed. Upload parts concurrently where possible |
| ECDSA signing compatibility with KSeF | Medium | Low | Behind feature flag, RSA remains default. Test against sandbox when enabled |
| Permission model complexity (7 types × CRUD) | Medium | Medium | All permission types share common patterns — extract shared HTTP helpers. Type-safe enums prevent mixups |
| Offline mode deadline enforcement across restarts | Medium | Medium | Persist offline invoices + deadlines in DB. On startup, check for expired offline invoices |
| `qrcode` + `image` crate compilation time | Low | Low | Feature-gate QR generation. Can be disabled for library-only users |

---

## Open Questions

- [ ] What are the exact XML schema namespaces for each invoice type (Kor, Zal, Roz, etc.)?
- [ ] Does KSeF batch upload require specific part size limits beyond the default 5MB?
- [ ] What's the exact format of KSeF KOD II signature — is it RSA-SHA256 or does it depend on certificate type?
- [ ] Are offline invoice QR codes required before KSeF mandatory rollout date, or is this a post-mandate feature?
- [ ] What are the production rate limits? (Can be queried dynamically, but good to know defaults)
- [ ] Does PEPPOL integration require additional credentials beyond the standard KSeF token?

---

## Decision Log

| Decision | Rationale | Alternatives Considered |
|----------|-----------|------------------------|
| Single ksef-core crate | Simpler DI, consistent with existing arch. Module boundaries provide enough isolation | Separate crates per domain (more compile isolation but wiring overhead) |
| Library + server equally important | ksef-core must work without Axum/DB; traits are the API boundary | Server-first (would leak HTTP types into domain) |
| In-memory rate limiter + DB tokens | Hot path (rate limiting) needs sub-ms latency. Cold path (tokens) needs persistence across restarts | Full DB (too slow for rate limiting), full in-memory (loses tokens on restart) |
| `zip` crate for batch | Most popular, pure Rust, well-maintained. Sync API is fine — ZIP creation is CPU-bound, not IO-bound | async-zip (less mature), manual impl (too much work) |
| `qrcode` + `image` for QR | Full PNG/SVG rendering in Rust. Dashboard can display directly. Users want printable QR codes | URL-only (requires frontend rendering), ASCII (not production quality) |
| ECDSA behind feature flag | Future-proofing without adding mandatory dependency. P-256 crate is lightweight | Always compile (unnecessary for RSA-only users), skip entirely (limits future compat) |
| Test data behind cfg(test) | No production binary bloat. Only needed in dev/test. Can't accidentally call in prod | Runtime guard (compiles dead code into prod), separate binary (extra build target) |
| Full permission types from start | User wants complete API coverage. All 7 types follow same pattern — marginal effort to add all vs subset | Basic-first (would need second pass), trait-only (stub implementations are tech debt) |
| Full invoice type parsers | Different types have genuinely different XML structures — generic parser would be fragile and incomplete | Generic parser (loses type-specific fields), domain-only (can't actually parse anything) |
| SSR Askama for dashboard | Consistent with existing stack. No JS build pipeline. Server-rendered is simpler for forms + tables | htmx (good but adds JS dependency), REST-only (loses dashboard value) |
| Sandbox E2E suite | Real API verification catches issues mocks can't. Gated behind env var so it doesn't slow CI | Mocks-only (misses real API quirks), always-on E2E (flaky CI from sandbox instability) |
