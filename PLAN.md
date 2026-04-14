# Multi-Tenant Auth & NIP Context — Implementation Plan

## Overview

Replace the single-NIP-from-env architecture with multi-tenant login: users register with email/password, add NIP accounts, and operate KSeF in the context of a selected NIP. This enables multiple users on one instance, each managing one or more NIPs (e.g. accounting firms managing clients).

## Goals

- User registration and login (NIP + password, argon2 hashing)
- Many-to-many user ↔ NIP relationship (one user can manage multiple NIPs)
- All KSeF operations scoped to the currently selected NIP account
- Per-NIP KSeF certificate storage (auto-gen for test/demo, upload for production)
- Background worker operates on jobs tagged with NIP, loads cert/token per NIP from DB
- HTTP sessions via tower-sessions with DB backend
- Clean URL routing: `/accounts/{nip}/invoices`, `/accounts/{nip}/sessions`, etc.
- Remove `KSEF_NIP` from .env entirely

## Non-Goals

- OAuth / SSO / login.gov.pl integration (future)
- Role-based access control within an account (all users with NIP access are equal)
- Multi-worker parallelism (one worker is sufficient, scales later via `FOR UPDATE SKIP LOCKED`)
- Data migration from current single-NIP schema (clean start)

## Engineering Principles (Non-Negotiable)

These principles govern every line of code in this plan. They are constraints, not suggestions.

1. **Protocol first** — define traits, API contracts, and type signatures BEFORE implementation. Every new feature starts with the port trait, not the HTTP handler or DB query. `UserRepository` trait before `PgUserRepo`. `NipAccountRepository` trait before queries. Auth middleware type signature before session store wiring.
2. **Test-driven development** — write the failing test first, then implement. Tests ARE the spec. No implementation without a corresponding test. Contract test suites for port traits validate both mocks and real implementations against the same spec.
3. **Fail fast, no fallbacks** — errors surface immediately. No silent swallowing, no default fallbacks that mask bugs. `Result<T, E>` everywhere. Missing NIP context → 403, not redirect to fallback. Invalid session → 401, not anonymous access. Missing cert on production → startup error, not silent degradation.
4. **Clean architecture** — domain logic is independent of frameworks, DB, HTTP. Dependencies point inward: `domain ← ports ← services ← infra`. No Axum types in domain. No SQL in services. `AuthUser` and `NipAccount` are domain types. `tower-sessions` is infra.
5. **Idiomatic Rust** — newtype wrappers for `UserId`, `NipAccountId`. `thiserror` for error hierarchies. Ownership over cloning. Builder patterns where construction is complex.
6. **Clean semantics** — names precisely reflect intent. No ambiguous abbreviations. `NipContext` not `Ctx`. `UserRepository` not `UserStore`. Code reads like documentation.

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

## Assumptions and Constraints

- Clean database — no migration of existing test data
- `KSEF_ENVIRONMENT` stays in .env (server-wide, not per-NIP) — all NIPs on one instance use the same KSeF environment
- `KSEF_CERT_PEM` / `KSEF_KEY_PEM` removed from .env — certs stored per-NIP in DB or auto-generated
- tower-sessions crate for session management (MIT, well-maintained, Axum-native)
- argon2 crate for password hashing
- SSR stays (Askama) — no SPA rewrite

## Requirements

### Functional

- Register: email + password → create user
- Login: email + password → session cookie
- Logout: destroy session
- Add NIP account: NIP + optional cert upload → creates account, auto-gen cert if test/demo
- Remove NIP account
- Switch NIP context: `/accounts` page shows all NIPs, click → enter NIP context
- All existing features (invoices, fetch, sessions, tokens, permissions, export) work under `/accounts/{nip}/...`
- Background worker resolves NIP from job payload, loads cert/session per NIP

### Non-Functional

- Passwords hashed with argon2id (OWASP recommendation)
- Session cookie: HttpOnly, SameSite=Lax, Secure in production
- NIP data isolated: user can only see NIPs they own
- Cert private keys encrypted at rest in DB (AES-256-GCM with server-side key from env)

## Technical Design

### Data Model

New tables (both PG and SQLite):

```sql
-- User accounts
CREATE TABLE users (
    id TEXT PRIMARY KEY,           -- UUID
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,   -- argon2id
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- NIP accounts owned by users
CREATE TABLE nip_accounts (
    id TEXT PRIMARY KEY,           -- UUID
    nip TEXT NOT NULL,
    display_name TEXT NOT NULL,    -- e.g. "Moja firma" or company name
    ksef_auth_method TEXT NOT NULL DEFAULT 'xades',  -- 'xades' or 'token'
    ksef_auth_token TEXT,          -- if auth_method = token
    cert_pem TEXT,                 -- PEM cert (encrypted at rest)
    key_pem TEXT,                  -- PEM key (encrypted at rest)
    cert_auto_generated INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(nip)                    -- one account per NIP globally
);

-- Many-to-many: user ↔ NIP
CREATE TABLE user_nip_access (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nip_account_id TEXT NOT NULL REFERENCES nip_accounts(id) ON DELETE CASCADE,
    granted_at TEXT NOT NULL,
    PRIMARY KEY (user_id, nip_account_id)
);

-- Session storage for tower-sessions
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    data BLOB NOT NULL,
    expiry_date TEXT NOT NULL
);
```

Modified tables:
```sql
-- invoices: add nip_account_id foreign key
ALTER TABLE invoices ADD COLUMN nip_account_id TEXT NOT NULL REFERENCES nip_accounts(id);
CREATE INDEX idx_invoices_nip_account ON invoices(nip_account_id);

-- jobs: add nip (denormalized for worker simplicity)
-- already has payload JSON with invoice_id, add nip field
ALTER TABLE jobs ADD COLUMN nip TEXT;

-- ksef_auth_tokens: already has nip column — no change needed
-- ksef_sessions: already has nip column — no change needed
```

### URL Routing

```
/                           → redirect to /accounts (if logged in) or /login
/register                   → registration form
/login                      → login form
/logout                     → destroy session, redirect to /login
/accounts                   → list of NIP accounts for current user
/accounts/add               → add NIP account form
/accounts/{nip}             → dashboard for this NIP (current /)
/accounts/{nip}/invoices    → invoice list (current /invoices)
/accounts/{nip}/invoices/new
/accounts/{nip}/invoices/{id}
/accounts/{nip}/invoices/{id}/submit
/accounts/{nip}/invoices/fetch
/accounts/{nip}/sessions
/accounts/{nip}/permissions
/accounts/{nip}/tokens
/accounts/{nip}/export
/accounts/{nip}/settings    → NIP account settings (cert upload, auth method)
```

### Architecture

```
Request
  │
  ├── Auth middleware (tower-sessions)
  │     └── extracts user_id from session cookie
  │
  ├── NIP context extractor (from path :nip)
  │     └── verifies user has access to this NIP
  │     └── loads NipAccount with cert/config
  │
  ├── Route handler
  │     └── uses NipContext { user, nip_account, nip }
  │     └── passes nip to services
  │
  └── Services (unchanged signatures, NIP passed per-call)
        └── SessionService.ensure_token(&nip)
        └── FetchService.fetch_invoices(&nip, &query)
        └── etc.
```

### Key Types

```rust
/// Extracted from session cookie by auth middleware.
pub struct AuthUser {
    pub id: UserId,
    pub email: String,
}

/// Extracted from URL path + verified against user access.
pub struct NipContext {
    pub user: AuthUser,
    pub account: NipAccount,
}

/// Stored NIP account with KSeF credentials.
pub struct NipAccount {
    pub id: NipAccountId,
    pub nip: Nip,
    pub display_name: String,
    pub auth_method: AuthMethod,
    pub cert_pem: Option<Vec<u8>>,  // decrypted
    pub key_pem: Option<Vec<u8>>,   // decrypted
}
```

---

## Implementation Plan

### Serial Dependencies (Must Complete First)

#### Phase 0: Data Model & Auth Foundation
**Prerequisite for:** All subsequent phases

| Task | Description | Output |
|------|-------------|--------|
| 0.1 | Add new domain types: `UserId`, `NipAccountId`, `NipAccount`, `AuthUser` in `domain/` | New domain module `domain/auth_user.rs` and `domain/nip_account.rs` |
| 0.2 | Create new migration 005 with `users`, `nip_accounts`, `user_nip_access`, `sessions` tables (PG + SQLite) | Migration files |
| 0.3 | Add `nip_account_id` column to `invoices` table, `nip` column to `jobs` table in migration 005 | Same migration |
| 0.4 | Create port traits: `UserRepository`, `NipAccountRepository` | New files in `ports/` |
| 0.5 | Implement repos for both PG and SQLite | New query modules in `infra/pg/queries/` and `infra/sqlite/queries/` |
| 0.6 | Add argon2 + tower-sessions dependencies to Cargo.toml | Updated Cargo.toml |

#### Phase 1: Session & Auth Middleware
**Prerequisite for:** All route changes

| Task | Description | Output |
|------|-------------|--------|
| 1.1 | Implement tower-sessions DB store for SQLite and PG (or use existing `tower-sessions-sqlx-store`) | Session store wired into Axum |
| 1.2 | Create auth middleware: extracts `AuthUser` from session, rejects unauthenticated requests (except `/login`, `/register`) | `server/src/middleware/auth.rs` |
| 1.3 | Create NIP context extractor: reads `{nip}` from path, verifies user access, loads `NipAccount` | `server/src/extractors/nip_context.rs` |
| 1.4 | Create `register` + `login` + `logout` route handlers and templates | `routes/auth.rs`, `templates/pages/login.html`, `register.html` |

---

### Parallel Workstreams

#### Workstream A: Account Management UI
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** Workstream B, C

| Task | Description | Output |
|------|-------------|--------|
| A.1 | `/accounts` page — list NIP accounts for current user | Route handler + template |
| A.2 | `/accounts/add` — form: NIP, display name, cert upload (optional on test/demo) | Route handler + template |
| A.3 | `/accounts/{nip}/settings` — view/edit NIP account config, cert upload/replace | Route handler + template |
| A.4 | Auto-generate cert when adding NIP on test/demo environment | Logic in NipAccount service |

#### Workstream B: Route Migration
**Dependencies:** Phase 0, Phase 1
**Can parallelize with:** Workstream A, C

| Task | Description | Output |
|------|-------------|--------|
| B.1 | Move all existing routes under `/accounts/{nip}/` prefix | Updated `routes/mod.rs` with nested router |
| B.2 | Replace `State(state).nip` usage in all handlers with `NipContext.nip` from extractor | Update all route handlers |
| B.3 | Update all Askama templates: navbar shows current NIP + link to `/accounts`, all internal links prefixed | Updated templates |
| B.4 | Root `/` redirects to `/accounts` (or `/login` if unauthenticated) | Redirect handler |

#### Workstream C: Service & Worker Refactor
**Dependencies:** Phase 0
**Can parallelize with:** Workstream A, B

| Task | Description | Output |
|------|-------------|--------|
| C.1 | Remove global `nip` from `AppState` — services receive NIP per-call | Updated `state.rs`, service constructors |
| C.2 | `FetchService::fetch_invoices` takes `&Nip` parameter (already does) — verify all services follow pattern | Audit + fix any that don't |
| C.3 | `SessionService` loads signer (cert/key) per NIP from `NipAccountRepository` instead of global signer | Updated `SessionService` constructor/methods |
| C.4 | `JobWorker`: extract NIP from job payload, load cert per NIP, create per-NIP signer for each job | Updated `job_worker.rs` |
| C.5 | Invoice save/list queries filter by `nip_account_id` | Updated repo queries |
| C.6 | Remove `KSEF_NIP` from config, .env.example | Updated `config.rs`, `.env.example` |

---

### Merge Phase

#### Phase 2: Integration & Polish
**Dependencies:** Workstreams A, B, C

| Task | Description | Output |
|------|-------------|--------|
| 2.1 | Wire everything in `main.rs`: tower-sessions layer, auth middleware, new routes, DB session store | Updated `main.rs` |
| 2.2 | Update `db_backend.rs` to expose `UserRepository` + `NipAccountRepository` in `DatabasePorts` | Updated `DatabasePorts` struct |
| 2.3 | End-to-end manual test: register → login → add NIP → create invoice → submit → fetch | Verified flow |
| 2.4 | Update README with new auth/login flow, remove KSEF_NIP references | Updated README |

---

## Testing and Validation

- **Unit tests:** New domain types (UserId, NipAccountId), password hashing, NIP context extraction
- **Integration tests (SQLite):** User CRUD, NipAccount CRUD, access control queries, session store
- **Integration tests (PG):** Same suite via testcontainers
- **Service tests:** SessionService with per-NIP signer, JobWorker with per-NIP cert loading
- **Manual E2E:** Full flow from register to KSeF invoice submission

## Rollout and Migration

- **Clean break:** New migration creates tables, old data not migrated
- **No feature flags:** This replaces the entire auth model
- **Rollback:** Revert to previous commit — single-NIP mode is the prior state

## Verification Checklist

```sh
# 1. Compile
cargo check

# 2. Unit tests
cargo test -p ksef-core --lib

# 3. SQLite integration
cargo test -p ksef-core --test sqlite_integration

# 4. Server starts
cargo run -p ksef-server
# Should show login page at http://localhost:3000

# 5. Manual flow
# - Register at /register
# - Login at /login
# - Add NIP at /accounts/add
# - Navigate to /accounts/{nip}/invoices
# - Create and submit invoice
```

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Cert storage security (private keys in DB) | Med | High | Encrypt at rest with server-side key (AES-256-GCM), key from env var `CERT_ENCRYPTION_KEY` |
| Session fixation | Low | Med | tower-sessions handles session ID rotation on login |
| Multi-NIP worker cert loading adds latency | Low | Low | Cache loaded signers in-memory per NIP (LRU) |
| Breaking change — all routes change | High | Med | One-shot migration, no backward compat needed (pre-production) |

## Decision Log

| Decision | Rationale | Alternatives Considered |
|----------|-----------|------------------------|
| Multi-tenant with user auth | Target: accounting firms with multiple NIPs | Single-user with NIP-at-login (simpler but limited) |
| NIP + password auth | Simple, self-contained, no external IdP dependency | OAuth (complex), NIP-only (no security), PIN (weak) |
| One worker, NIP in job payload | Simple, robust, no lifecycle management | Worker-per-NIP (complex coordination) |
| tower-sessions + DB store | Server-side revocation, session visibility | Cookie store (stateless but no revocation), custom (more code) |
| `/accounts/{nip}/...` routing | Clean URL hierarchy, clear NIP context | Dropdown switcher (hides context in session, URLs don't reflect NIP) |
| Clean DB start | Pre-production test data, not worth migrating | Migrate to default account (extra complexity for no value) |
| `KSEF_NIP` fully removed from env | No duality, NIP comes from user accounts only | Keep as seed (extra code path, confusing) |
| Auto-gen cert on test/demo per NIP | Zero-friction development, matches current behavior | Always require upload (friction for test), token-only (expires 2026) |
