# KsefPay

Polish e-invoice (KSeF) integration in Rust. Send, receive, and manage VAT invoices through the national KSeF API v2.0.

> **Unofficial project.** Not affiliated with or endorsed by the Ministry of Finance of Poland. Use at your own risk.

## What is KSeF

KSeF (Krajowy System e-Faktur) is Poland's mandatory e-invoicing system run by the Ministry of Finance. The rollout is phased: large taxpayers from **1 February 2026**, remaining companies from **1 April 2026**, and previously exempt smallest entities from **1 January 2027**. All must submit VAT invoices as signed and encrypted FA(3) XML through the KSeF API.

KsefPay is a **standalone server** for micro and solo businesses that don't have an ERP and don't want to pay for invoicing SaaS. Fill out a form, the app generates FA(3) XML, encrypts it (AES-256-CBC + RSA-OAEP), signs the session (XAdES-BES), submits to KSeF, and tracks the status.

## Quick start

```sh
git clone https://github.com/michalpalkowski/KsefPay.git && cd KsefPay
cp .env.example .env
# .env defaults to PostgreSQL — for zero-dependency start, change to:
#   DATABASE_URL=sqlite://./.data/ksef.db
cargo run -p ksef-server
# → http://localhost:3000
```

Migrations run automatically on startup. Certificates are auto-generated for `test`/`demo` environments.

For PostgreSQL setup, Docker deployment, and detailed configuration see [Detailed setup](#detailed-setup).

## Dashboard

SSR web interface (Axum + Askama + Tailwind CSS).

<!-- TODO: add screenshot here -->

| Page | What it does |
|---|---|
| **Home** (`/`) | Invoice status summary — drafts, queued, accepted, errors |
| **Invoices** (`/invoices`) | Outgoing (sent) and incoming (received) invoices with detail view |
| **New invoice** (`/invoices/new`) | VAT invoice form — seller, buyer, line items, payment terms. Auto-calculates net/VAT/gross |
| **Sessions** (`/sessions`) | KSeF authentication status. One click to start the full challenge → XAdES sign → poll → JWT flow |
| **Permissions** (`/permissions`) | Grant/revoke access for other NIPs (e.g. accounting firm). 7 permission types |
| **Tokens** (`/tokens`) | Generate/revoke API tokens as an alternative to certificate auth |
| **Export** (`/export`) | Bulk export invoices from KSeF for a given period |

Submission to KSeF happens in the background — a job worker handles encryption, upload, retry with exponential backoff, and dead-letter queue for persistent failures.

Fetching invoices (`/invoices/fetch`) pulls incoming invoices from KSeF for a selected period, parses FA(3) XML, and stores them locally. Idempotent — re-fetching updates existing records without duplicates.

## Features

**Invoices**
- 12 invoice types: VAT, corrections, advance, split, simplified, proforma, and more
- FA(3) XML generation and parsing (quick-xml, roxmltree)
- AES-256-CBC + RSA-OAEP encryption
- Background submission with retry + dead-letter queue
- Fetch incoming invoices from KSeF (Subject2/Subject3 queries)
- Async export with polling

**Authentication**
- XAdES-BES signing (Exclusive XML Canonicalization)
- Full auth flow: challenge → sign → poll → JWT
- Token-based auth as alternative
- Auto-generated certificates for test/demo environments

**Management**
- Permission grant/revoke/query (7 permission types)
- API token lifecycle (generate, list, revoke)
- Batch invoice sessions with ZIP upload
- Offline invoice workflow + QR code generation (PNG/SVG)
- PEPPOL directory lookups
- Rate limit monitoring

**Validated against real KSeF** — full E2E flow tested on `api-test.ksef.mf.gov.pl`.

## Architecture

```
ksef-core/       Library crate — domain, ports, services, infra
ksef-server/     Axum web server + Askama templates (standalone binary)
```

Clean architecture with inward-pointing dependencies: `domain` ← `ports` ← `services` ← `infra` ← `server`.

```
ksef-core/src/
  domain/         Pure types: Invoice, Nip, Money, VatRate, auth, crypto, permissions
  ports/          Trait interfaces (16): repos, KSeF client, encryption, transactions
  services/       Business logic (9): invoice, session, fetch, batch, permission, token, export, offline, QR
  infra/
    pg/           PostgreSQL implementation
    sqlite/       SQLite implementation
    crypto/       XAdES signing, AES+RSA encryption
    fa3/          FA(3) XML serialization and parsing
    ksef/         HTTP clients for KSeF API (14 endpoint modules)
    http/         Rate limiter (token bucket), retry with exponential backoff
    batch/        ZIP archive builder
    qr/           QR code generation
  workers/        Background job processor

ksef-server/src/
  routes/         Axum handlers: dashboard, invoices, sessions, permissions, tokens, export, fetch
  templates/      Askama HTML templates (10 pages)
```

Dual database backend — PostgreSQL and SQLite, selected via `DATABASE_URL`. Both share the same port traits and migration schema.

## Tests

```sh
# Unit tests (313 tests, no external dependencies)
cargo test -p ksef-core --lib

# SQLite integration tests (no external DB needed)
cargo test -p ksef-core --test sqlite_integration

# PostgreSQL integration tests (needs running PostgreSQL)
cargo test -p ksef-core --test pg_integration

# KSeF sandbox E2E (needs network + certificate)
KSEF_E2E_CERT_PEM=.tmp/cert.pem KSEF_E2E_KEY_PEM=.tmp/key.pem \
  cargo test -p ksef-core --test ksef_e2e -- --ignored --nocapture --test-threads=1
```

## Detailed setup

### Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | yes | — | `sqlite://...` or `postgres://...` |
| `KSEF_ENVIRONMENT` | no | `test` | `test`, `demo`, or `production` |
| `KSEF_CERT_PEM` | prod only | auto | Path or inline PEM certificate |
| `KSEF_KEY_PEM` | prod only | auto | Path or inline PEM private key |
| `SERVER_HOST` | no | `0.0.0.0` | Bind address |
| `SERVER_PORT` | no | `3000` | HTTP port |
| `RUST_LOG` | no | `info` | Log filter |

### PostgreSQL setup

```sh
# Ubuntu/Debian
sudo apt install -y postgresql postgresql-contrib
sudo -u postgres psql -d postgres <<'SQL'
DO $$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'ksef') THEN
    CREATE ROLE ksef LOGIN PASSWORD 'ksef';
  END IF;
END $$;
SQL
sudo -u postgres createdb -O ksef ksef 2>/dev/null || true

# macOS
brew install postgresql@16
brew services start postgresql@16
createdb -O ksef ksef 2>/dev/null || true
```

### Docker (production)

```sh
docker compose -f docker-compose.prod.yml up --build
```

Multi-stage build: `rust:1.92-bookworm` → `debian:bookworm-slim`.

### KSeF sandbox registration

```sh
# Register test subject (idempotent)
cargo run -p ksef-core --example register_subject -- <YOUR_NIP>
```

Well-known NIP `5260250274` (Ministry of Finance) works out of the box on sandbox.

For certificate generation see [docs/test-cert-howto.md](docs/test-cert-howto.md).

## Tech stack

Rust 1.88+ · Axum 0.8 · Askama 0.15 · sqlx 0.8 (PostgreSQL + SQLite) · reqwest · OpenSSL · tokio · thiserror · tracing

## License

[MIT](LICENSE)
