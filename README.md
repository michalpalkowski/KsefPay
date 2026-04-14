# ksef-paymoney

Integracja z polskim KSeF (Krajowy System e-Faktur) w Rust. Wysyłanie, odbieranie i zarządzanie e-fakturami przez KSeF API v2.0.

## Czym jest KSeF

KSeF to rządowy system e-faktur prowadzony przez Ministerstwo Finansów. Od 2026 roku każda firma w Polsce musi wysyłać faktury VAT przez KSeF — nie mailem, nie PDF-em, tylko jako podpisany i zaszyfrowany XML w formacie FA(3). KSeF nadaje każdej fakturze unikalny numer i przechowuje ją w centralnej bazie.

Ten projekt to **samodzielny serwer** do obsługi KSeF, celowany w jednoosobowe i mikro-firmy, które nie mają systemu ERP ani nie chcą płacić za SaaS do fakturowania. Wypełniasz formularz, aplikacja generuje FA(3) XML, szyfruje go (AES-256-CBC + RSA-OAEP), podpisuje sesję (XAdES-BES), wysyła do KSeF i śledzi status.

## Architecture

```
ksef-core/       Library crate — domain, ports, services, infra
ksef-server/     Axum + Askama web dashboard (standalone binary)
```

Clean architecture: `domain` ← `ports` ← `services` ← `infra` ← `server`.
PostgreSQL for persistence. Background job worker for async KSeF operations.

## Dashboard

SSR (Askama + Tailwind CSS), interfejs po polsku. Po uruchomieniu: `http://localhost:3000`.

### Strona główna (`/`)

Podsumowanie stanu faktur w systemie: ile szkiców czeka na wysłanie, ile jest w kolejce do KSeF, ile zaakceptowanych, ile z błędami. Daje szybki obraz tego, co wymaga uwagi.

### Faktury (`/invoices`)

Dwie zakładki: **Wystawione** (faktury sprzedaży, które wysłałeś do KSeF) i **Otrzymane** (faktury zakupu pobrane z KSeF, np. od dostawców). Tabela pokazuje numer faktury, datę, kontrahenta, kwotę brutto, status i numer KSeF (nadawany po akceptacji).

**Nowa faktura** (`/invoices/new`) — formularz do wystawienia faktury VAT. Wypełniasz dane sprzedawcy (NIP wstawiany z konfiguracji), nabywcy, pozycje (opis, ilość, cena netto, stawka VAT) i warunki płatności. Aplikacja generuje z tego XML w formacie FA(3) i zapisuje jako szkic. W tle liczy netto/VAT/brutto.

**Wysyłka do KSeF** — na szczegółach faktury (status = draft) klikasz "Wyślij do KSeF". Faktura trafia do kolejki. Background worker automatycznie: otwiera sesję z KSeF, szyfruje XML kluczem publicznym ministerstwa, wysyła, odbiera numer KSeF i UPO (Urzędowe Poświadczenie Odbioru). Jeśli coś się nie powiedzie — retry z exponential backoff.

**Pobieranie z KSeF** (`/invoices/fetch`) — ściąganie faktur zakupowych (lub własnych) z KSeF za wybrany okres. Np. "pobierz wszystkie faktury otrzymane w ostatnim miesiącu". Aplikacja odpytuje API KSeF, parsuje FA(3) XML każdej faktury i zapisuje w bazie. Idempotentne — powtórne pobranie aktualizuje istniejące, nie duplikuje.

### Sesje (`/sessions`)

KSeF wymaga uwierzytelnienia przed jakąkolwiek operacją. Flow: challenge → podpis XAdES certyfikatem → polling statusu → JWT token. Ta strona pokazuje czy masz aktywny token i otwartą sesję online. "Uwierzytelnij" startuje cały flow automatycznie. Na środowiskach test/demo certyfikat generuje się sam — nie musisz niczego konfigurować.

### Uprawnienia (`/permissions`)

KSeF ma system uprawnień — możesz nadać innemu NIP-owi (np. biurowi rachunkowemu) prawo do odczytu lub wystawiania faktur w Twoim imieniu. 7 typów uprawnień: odczyt faktur, wystawianie, zarządzanie uprawnieniami, introspekcja, podjednostki, odczyt uprawnień, operacje egzekucyjne. Tu możesz nadawać, odbierać i sprawdzać kto ma jakie uprawnienia do Twojego NIP-u.

### Tokeny (`/tokens`)

Alternatywa dla certyfikatu — możesz wygenerować token API z wybranymi uprawnieniami i używać go do uwierzytelniania (np. z innego systemu). Lista tokenów z ich statusem (aktywny/wygasły/unieważniony). Token raz unieważniony nie może być przywrócony.

### Eksport (`/export`)

Zbiorczy eksport faktur z KSeF za wybrany okres — przydatne do archiwizacji albo przekazania do biura rachunkowego. KSeF generuje plik asynchronicznie, aplikacja polluje status i udostępnia link do pobrania gdy gotowy.

## Features (ksef-core)

**Invoices**
- 12 invoice types: VAT, corrections (Kor), advance (Zal), split (Roz), simplified (Upr), proforma, and more
- FA(3) XML generation and parsing
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

## Quick start

```sh
# 1. Start PostgreSQL
docker compose up -d

# 2. Generate a test NIP (or use your own)
cargo run -p ksef-core --example gen_nip
# → 4583009462

# 3. Create .env
cp .env.example .env
# Set KSEF_NIP to your generated NIP

# 4. Run
cargo run -p ksef-server
# Dashboard at http://localhost:3000
# Certificate auto-generated for test/demo environments
```

Minimum `.env`:
```
DATABASE_URL=postgres://ksef:ksef@localhost:5432/ksef
KSEF_NIP=4583009462
```

### KSeF sandbox setup

Before sending invoices, the NIP must be registered on the KSeF test sandbox:

```sh
# Register test subject (idempotent — safe to run multiple times)
cargo run -p ksef-core --example register_subject -- 4583009462
```

Well-known NIP `5260250274` (Ministerstwo Finansów) works out of the box on sandbox.

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | yes | — | PostgreSQL connection string |
| `KSEF_NIP` | yes | — | Your company NIP (10 digits) |
| `KSEF_ENVIRONMENT` | no | `test` | `test`, `demo`, or `production` |
| `KSEF_AUTH_METHOD` | no | `xades` | `xades` or `token` |
| `KSEF_AUTH_TOKEN` | if token | — | Bootstrap token (when `auth_method=token`) |
| `KSEF_CERT_PEM` | prod only | auto | Path to PEM certificate |
| `KSEF_KEY_PEM` | prod only | auto | Path to PEM private key |
| `SERVER_HOST` | no | `0.0.0.0` | Bind address |
| `SERVER_PORT` | no | `3000` | HTTP port |
| `RUST_LOG` | no | `info` | Log filter (`info,ksef_core=debug,ksef_server=debug`) |

Certificate is auto-generated for `test`/`demo` environments.
Production requires a qualified electronic signature or KSeF certificate (Type I from MCU).
See [docs/test-cert-howto.md](docs/test-cert-howto.md) for details.

## Tests

```sh
# Unit tests (~148 tests, no dependencies)
cargo test -p ksef-core --lib

# PostgreSQL integration tests (needs Docker)
cargo test -p ksef-core --test pg_integration

# KSeF sandbox E2E (needs network + cert)
KSEF_E2E_CERT_PEM=.tmp/cert.pem KSEF_E2E_KEY_PEM=.tmp/key.pem \
  cargo test -p ksef-core --test ksef_e2e -- --ignored --nocapture --test-threads=1
```

E2E overrides: `KSEF_E2E_ENV`, `KSEF_E2E_NIP`.

## Docker (production)

```sh
docker compose -f docker-compose.prod.yml up --build
```

Multi-stage build: `rust:1.92-bookworm` → `debian:bookworm-slim`. Final image contains the binary, templates, and assets.

## Project structure

```
ksef-core/src/
  domain/         Pure types: Invoice, Nip, Money, VatRate, auth, crypto, permissions, tokens
  ports/          Trait interfaces (16): repos, KSeF client, encryption, transactions
  services/       Business logic (9): invoice, session, fetch, batch, permission, token, export, offline, QR
  infra/
    pg/           PostgreSQL repos + transactional Db/Tx
    crypto/       XAdES signing, AES+RSA encryption
    fa3/          FA(3) XML serialization and parsing
    ksef/         HTTP clients for all KSeF API endpoints (14 modules)
    batch/        ZIP archive builder
    qr/           QR code generation
    http/         Rate limiter (token bucket), retry with exponential backoff
  workers/        Background job processor

ksef-server/src/
  routes/         Axum handlers: dashboard, invoices, sessions, permissions, tokens, export, fetch
  templates/      Askama HTML templates (10 pages)
  assets/         CSS
```

## Tech stack

Rust 1.88+, Axum 0.8, Askama 0.15, sqlx (PostgreSQL 17), reqwest, OpenSSL, bergshamra-c14n (XAdES), thiserror, tokio, tracing.

## License

MIT
