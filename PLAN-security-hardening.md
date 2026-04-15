# KsefPay Security Hardening — Implementation Plan

## Overview

Fix critical authorization bypass vulnerabilities, enforce tenant isolation at the database level via `nip_account_id` FK, add CSRF protection, harden session cookies, implement rate limiting, and add audit logging. This is a production blocker — no deploy to real companies until complete.

## Goals
- Eliminate all CRITICAL and HIGH vulnerabilities from the security audit
- Enforce tenant isolation at the database level, not just string matching
- Make unauthorized data access structurally impossible (type system + DB constraints)
- Add defense-in-depth: CSRF tokens, session hardening, rate limiting, audit trail

## Non-Goals
- Email-based password reset (backlog — CLI reset covers MVP)
- Per-field encryption of invoice data at rest
- OAuth/SSO integration
- Penetration testing by external firm (do after hardening)

## Engineering Principles

These apply to every task in this plan. No exceptions.

- **Protocol first** — Define port traits and domain types before writing implementations. `AuditLogRepository`, `CsrfTokenStore` etc. are traits first, concrete later. Tests are written against the trait, not the impl.
- **Fail fast, no workarounds** — If `nip_account_id` is missing, the code must not compile (required field, not `Option`). If CSRF token is missing, reject with 403 — no "skip if absent" fallback. If rate limit is hit, return 429 immediately — no queuing.
- **Clean architecture** — Domain types have zero dependencies on infra. Ports define contracts. Services orchestrate. Infra implements. The server wires. No leaking of SQL or HTTP types into domain/ports.
- **TDD** — For every new port/service: write contract tests first (against mock), then integration tests (against real DB), then implement. Red → green → refactor. Tests prove the security boundary holds before the code ships.
- **No defaults that hide bugs** — No `Default` on security-critical types. `InvoiceFilter` already enforces this (requires `NipAccountId`). Same pattern for CSRF tokens, audit entries, rate limit keys.

## Assumptions and Constraints
- Production deploy planned within weeks — all phases are blockers
- Fly.io provides HTTPS termination; Caddy for local dev HTTPS
- SQLite is primary backend for now; PostgreSQL parity maintained
- Existing data in sandbox needs backfill migration for nip_account_id

## Requirements

### Functional
- Every invoice must be linked to a `nip_account_id` (NOT NULL FK)
- Invoice detail, submit, and export endpoints must verify ownership
- All POST forms must include and validate CSRF tokens
- Login/register endpoints must be rate-limited
- Sensitive operations must be recorded in an audit log

### Non-Functional
- Session cookies: HttpOnly, Secure, SameSite=Strict
- Rate limit: 10 requests/minute per IP on auth endpoints
- Audit log: append-only, queryable by user_id and nip
- Zero downtime migration for existing data

## Technical Design

### Data Model Changes

```sql
-- Migration 007: security hardening

-- 1. Audit log (append-only)
CREATE TABLE audit_log (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    user_id TEXT NOT NULL,
    user_email TEXT NOT NULL,
    nip TEXT,
    action TEXT NOT NULL,
    details TEXT,
    ip_address TEXT
);
CREATE INDEX idx_audit_log_user ON audit_log(user_id);
CREATE INDEX idx_audit_log_nip ON audit_log(nip);
CREATE INDEX idx_audit_log_timestamp ON audit_log(timestamp);

-- 2. Enforce nip_account_id on invoices (backfill script runs first)
-- After backfill: ALTER TABLE invoices ... NOT NULL (SQLite requires table rebuild)
```

### Architecture

```
Request → Rate Limiter → Session Layer (Secure/HttpOnly/SameSite)
       → CSRF Middleware (validate token on POST)
       → NipContext extractor (auth + access control)
       → Route handler (uses nip_account_id for all queries)
       → Audit log (writes to DB on sensitive operations)
```

---

## Implementation Plan

### Serial Dependencies (Must Complete First)

#### Phase 0: nip_account_id enforcement on Invoice model
**Prerequisite for:** All workstreams (changes domain model and all queries)

| Task | Description | Output |
|------|-------------|--------|
| 0.1 | Add `nip_account_id: NipAccountId` to `Invoice` domain struct. Make it required (not Option). Update `InvoiceId::new()` to accept it. | `domain/invoice.rs` updated |
| 0.2 | Add `nip_account_id` column handling to PG and SQLite `save()`, `upsert_by_ksef_number()`, `find_by_id()`, `list()` queries. Filter `find_by_id()` by `nip_account_id` (not just UUID). | Both backends updated |
| 0.3 | Update `InvoiceService::create_draft()` to accept `NipAccountId` and set it on the invoice. Update `InvoiceService::find()` to require `NipAccountId` and pass to repo. | Service layer enforces ownership |
| 0.4 | Update `FetchService::fetch_invoices()` to accept `NipAccountId` and set it on fetched invoices during upsert. | Incoming invoices get account linkage |
| 0.5 | Update all route handlers (`invoices::list`, `invoices::detail`, `invoices::create`, `invoices::submit`, `fetch::fetch_execute`) to pass `nip_ctx.account.id` to service methods. | Routes pass account context |
| 0.6 | Write migration 007: add NOT NULL constraint on `nip_account_id` (after backfill). For SQLite this requires table rebuild (CREATE new → INSERT SELECT → DROP old → ALTER RENAME). | Migration SQL for both backends |
| 0.7 | Write backfill script: `examples/backfill_nip_account.rs`. Iterates all invoices with NULL `nip_account_id`, matches `seller_nip` (outgoing) or `buyer_nip` (incoming) to `nip_accounts.nip`, sets FK. Supports `--dry-run`. Logs unmatched invoices. | Executable backfill tool |
| 0.8 | Update `InvoiceFilter` — remove `account_nip: Nip`, replace with `account_id: NipAccountId`. Update both backends and all callers. | Filter by FK, not string |
| 0.9 | Update all unit and integration tests. Add test: creating invoice without nip_account_id fails. Add test: find_by_id with wrong account_id returns NotFound. | Full test coverage for ownership |

---

### Parallel Workstreams

These workstreams can be executed independently after Phase 0.

#### Workstream A: Session hardening + CSRF
**Dependencies:** Phase 0 (for audit log integration, but can start cookie config immediately)
**Can parallelize with:** Workstreams B, C, D

| Task | Description | Output |
|------|-------------|--------|
| A.1 | Configure session cookie: `HttpOnly(true)`, `Secure(true)`, `SameSite(Strict)`, `Path("/")`. Apply in `main.rs` SessionManagerLayer. HTTPS required everywhere (Caddy for local dev, Fly.io for prod). | `main.rs` session config hardened |
| A.2 | Add CSRF token generation to session: on every GET that renders a form, generate a random token, store in session, pass to template. Create `CsrfToken` extractor. | `extractors.rs` + new `csrf.rs` module |
| A.3 | Add hidden `<input name="_csrf" value="{{ csrf_token }}">` field to ALL form templates (login, register, invoice_new, fetch, export, permissions, tokens, sessions, account_add, profile password). | All 12+ templates updated |
| A.4 | Add CSRF validation middleware or extractor: on every POST, read `_csrf` from form body, compare to session token. Reject with 403 on mismatch. | Middleware in `main.rs` or per-handler extractor |
| A.5 | Tests: CSRF token present in rendered forms. POST without token returns 403. POST with wrong token returns 403. POST with valid token succeeds. | Integration test coverage |

#### Workstream B: Export key isolation
**Dependencies:** Phase 0 (needs NipAccountId available)
**Can parallelize with:** Workstreams A, C, D

| Task | Description | Output |
|------|-------------|--------|
| B.1 | Change `ExportKeyStore` key from `String` (reference) to `(NipAccountId, String)` (account + reference). | `state.rs` type change |
| B.2 | Update `export::start_export` to store key with `(nip_ctx.account.id, reference)`. | `export.rs` store path |
| B.3 | Update `export::download` to look up key with `(nip_ctx.account.id, reference)`. If key not found for this account → 404. | `export.rs` download path |
| B.4 | Same pattern for `FetchJobStore` — key by `NipAccountId`, not NIP string. | `state.rs` + `fetch.rs` |
| B.5 | Test: User A starts export, User B cannot download it. User A can download their own. | Test coverage |

#### Workstream C: Rate limiting
**Dependencies:** None (independent middleware)
**Can parallelize with:** Workstreams A, B, D

| Task | Description | Output |
|------|-------------|--------|
| C.1 | Add `tower-governor` or in-memory token-bucket rate limiter. Configure: `/login` and `/register` POST → 10 req/min per IP. | New dependency + middleware |
| C.2 | Apply rate limit layer to auth routes only (not all routes). Return 429 Too Many Requests with Retry-After header. | `main.rs` route layering |
| C.3 | Test: 11th login attempt within 1 minute returns 429. After cooldown, succeeds again. | Rate limit tests |

#### Workstream D: Audit logging
**Dependencies:** Phase 0 (for nip_account_id in audit entries)
**Can parallelize with:** Workstreams A, B, C

| Task | Description | Output |
|------|-------------|--------|
| D.1 | Create `AuditLog` domain type and `AuditLogRepository` port trait with `log()` method. | `domain/audit.rs` + `ports/audit_log.rs` |
| D.2 | Implement for SQLite and PostgreSQL. INSERT-only, no update/delete. | `infra/sqlite/queries/audit.rs` + PG equivalent |
| D.3 | Create `AuditService` with typed actions: `Login`, `Register`, `CreateInvoice`, `SubmitInvoice`, `FetchInvoices`, `GrantPermission`, `RevokePermission`, `GenerateToken`, `RevokeToken`, `ExportStart`, `ChangePassword`. | `services/audit_service.rs` |
| D.4 | Inject `AuditService` into route handlers. Log at point of success (after operation completes, not before). Include user_id, email, nip, action, IP address. | All sensitive route handlers updated |
| D.5 | Migration 007: create `audit_log` table with indexes. | SQL migration |
| D.6 | Admin view: `/admin/audit` page to view recent audit log entries (authenticated, admin-only or per-NIP filtered). | Optional — can defer to CLI query |

#### Workstream E: Password reset CLI + misc
**Dependencies:** None
**Can parallelize with:** All

| Task | Description | Output |
|------|-------------|--------|
| E.1 | Create `examples/reset_password.rs`: accepts email, generates new random password, hashes with argon2, updates DB, prints new password to stdout. | Executable CLI tool |
| E.2 | Fix remaining Polish diacritic in `extractors.rs`: "Brak dostepu" → "Brak dostępu", "nieprawidlowy NIP" → "nieprawidłowy NIP", "blad repozytorium" → "błąd repozytorium". | `extractors.rs` updated |
| E.3 | Remove dead code: `FetchResultsTemplate`, `FetchErrorDisplay` struct, `fetch_results.html` template (all unused after background fetch refactor). | Dead code cleanup |

---

### Merge Phase

After parallel workstreams complete, these tasks integrate the work.

#### Phase F: Integration & verification
**Dependencies:** All workstreams

| Task | Description | Output |
|------|-------------|--------|
| F.1 | Run full test suite: `cargo test -p ksef-core --lib`, `cargo test -p ksef-core --test sqlite_integration`, `cargo test -p ksef-server`. All green. | CI-passing codebase |
| F.2 | Run backfill script on sandbox data: `cargo run --example backfill_nip_account -- --dry-run`, review output, then run for real. | All invoices have nip_account_id |
| F.3 | Manual QA on Fly.io: register, add NIP, create invoice, fetch invoices, export, permissions, tokens. Verify CSRF tokens in forms, verify isolation between NIPs. | QA sign-off |
| F.4 | Attempt cross-account access manually: try to access `/accounts/{other_nip}/invoices/{uuid}` — must get 403. Try export download with wrong NIP — must get 404. | Penetration test basics |
| F.5 | Review audit_log table after QA: verify all operations were logged with correct user/nip/action. | Audit trail verified |

---

## Testing and Validation

### Unit tests (ksef-core --lib)
- InvoiceFilter requires NipAccountId (compile-time)
- find_by_id with wrong account_id → NotFound
- MockInvoiceRepo respects nip_account_id
- AuditService logs correctly

### Integration tests (sqlite_integration)
- Invoice created with nip_account_id, queried by correct account → found
- Invoice queried by wrong account → not found
- Backfill script matches correctly
- Audit entries written and queryable

### Manual / E2E
- CSRF: submit form without token → 403
- Rate limit: 11 rapid logins → 429
- Cross-account: User A cannot see User B invoices
- Export: User A cannot download User B export
- Session: cookie has HttpOnly, Secure, SameSite=Strict flags

---

## Rollout and Migration

1. **Before deploy:** Run backfill script on sandbox: `cargo run --example backfill_nip_account -- --dry-run`
2. **Deploy migration 007:** Creates audit_log table, rebuilds invoices table with NOT NULL nip_account_id
3. **Deploy application:** All routes enforce nip_account_id, CSRF, session hardening
4. **After deploy:** Verify audit_log is populated, test cross-account access returns 403
5. **Rollback plan:** Migration 007 is additive (new table + constraint). Rollback = deploy previous image (Fly.io), constraint doesn't break old code since old code never queries without NIP context.

---

## Verification Checklist

```sh
# 1. All tests pass
cargo test -p ksef-core --lib
cargo test -p ksef-core --test sqlite_integration
cargo test -p ksef-server

# 2. No compiler warnings
cargo build -p ksef-server 2>&1 | grep "warning"

# 3. Backfill dry-run
cargo run --example backfill_nip_account -- --dry-run

# 4. CSRF token present in form
curl -s https://ksef-paymoney.fly.dev/login | grep "_csrf"

# 5. POST without CSRF rejected
curl -s -X POST https://ksef-paymoney.fly.dev/login -d "email=x&password=y" -w "%{http_code}"
# Expected: 403

# 6. Rate limit triggered
for i in $(seq 1 12); do curl -s -o /dev/null -w "%{http_code}\n" -X POST https://ksef-paymoney.fly.dev/login -d "email=x&password=y"; done
# Expected: 429 after ~10 requests

# 7. Session cookie flags
curl -sI https://ksef-paymoney.fly.dev/login | grep -i set-cookie
# Expected: HttpOnly; Secure; SameSite=Strict
```

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Backfill script mismatches invoices to wrong accounts | Low | Critical | Dry-run mode, manual review of unmatched, reversible (NULL → re-run) |
| SQLite table rebuild migration fails on large DB | Low | High | Test on copy of production DB first. Keep backup. |
| CSRF tokens break existing browser sessions | Medium | Low | Users just need to re-login. Session expiry handles cleanup. |
| Rate limiter blocks legitimate users behind shared IP (NAT) | Low | Medium | 10/min is generous. Add X-Forwarded-For support for proxy setups. |
| nip_account_id NOT NULL blocks invoice creation if account missing | Low | High | FetchService creates/finds account before inserting invoices |

## Open Questions

- [ ] Should audit log be visible in UI (admin page) or only via DB query?
- [ ] Do we need account-level (not just user-level) rate limiting?
- [ ] Should CSRF token rotate per-request or per-session?

## Decision Log

| Decision | Rationale | Alternatives Considered |
|----------|-----------|------------------------|
| nip_account_id FK over NIP string matching | String matching allows spoofing if user creates account with matching NIP. FK is structural guarantee. | NIP string filter (current, vulnerable) |
| Full CSRF tokens over SameSite-only | SameSite doesn't protect against same-site subdomain attacks. Tokens are defense-in-depth for production. | SameSite=Strict only (simpler) |
| Audit log in DB over structured logs | DB audit log is queryable, doesn't depend on log retention policy, survives log rotation. | tracing logs (less durable), skip (not acceptable for production) |
| Secure cookie always (not conditional) | Enforces HTTPS everywhere. Caddy handles local dev. No "forgot to switch" risk in production. | Conditional on environment (error-prone) |
| Backfill via Rust script over SQL migration | Rust script can validate, dry-run, and handle edge cases. SQL migration is one-shot and opaque. | SQL UPDATE (simpler but riskier), drop and re-fetch (destructive) |
