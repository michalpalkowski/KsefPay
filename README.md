# KsefPay

Polish e-invoice (KSeF) integration in Rust. Send, receive, and manage VAT invoices through the national KSeF API v2.0.

> Unofficial project. Not affiliated with or endorsed by the Ministry of Finance of Poland.

## What is this

KsefPay is a standalone web application for working with KSeF:

- create and send FA(3) invoices
- fetch incoming invoices
- manage sessions, permissions, tokens and exports
- manage many workspaces and many NIP accounts
- invite users either:
  - to a shared workspace
  - or to the application with their own independent workspace

The app is now `workspace-first`:

- data isolation is done by workspace
- sharing a workspace shares its data
- application access and workspace access are two different flows

## Local start

The simplest real local start is:

```sh
cp .env.example .env
docker compose up -d postgres mailpit
cargo run -p ksef-server
```

After start:

- app: `http://127.0.0.1:3000`
- local email inbox: `http://127.0.0.1:8025`

If you want one shortcut command, you can also use:

```sh
cp .env.example .env
make dev
```

`make dev` just does the same thing more conveniently:

- starts local PostgreSQL
- starts Mailpit for local emails
- runs the server

## Local setup step by step

### Requirements

- Rust toolchain
- Docker with `docker compose`
- OpenSSL available in system

### 1. Prepare `.env`

```sh
cp .env.example .env
```

For local dev the default `.env.example` is already usable.

Important values:

- `DATABASE_URL=postgres://ksef:ksef@localhost:5432/ksef`
- `APP_BASE_URL=http://localhost:3000`
- `KSEF_ENVIRONMENT=test`
- `APPLICATION_ACCESS_MODE=email_invite`
- `SMTP_HOST=127.0.0.1`
- `SMTP_PORT=1025`
- `SMTP_SECURITY=plaintext`
- `SMTP_AUTH=none`
- `ALLOWED_EMAILS=admin@example.com`

Change `ALLOWED_EMAILS` to your bootstrap admin email before first login, for example:

```env
ALLOWED_EMAILS=your_email@example.com
```

### 2. Start dependencies and app

```sh
docker compose up -d postgres mailpit
cargo run -p ksef-server
```

Optional shortcut:

```sh
make dev
```

### 3. Register the first admin

Open:

- `http://127.0.0.1:3000/register`

Register using an email listed in `ALLOWED_EMAILS`.

That creates:

- the first user account
- the user’s own independent workspace

### 4. Invite other users correctly

There are now two separate flows:

#### A. Share current workspace

Use:

- `Workspace`

Effect:

- invited user joins the current workspace
- invited user sees its data according to role
- this includes shared NIP accounts, invoices and related resources

#### B. Give access only to the application

Use:

- `Dostęp do aplikacji`

Effect:

- invited user gets access to KsefPay
- invited user does not join your workspace
- invited user creates and uses their own independent workspace

### 4a. Application Access Mode

Application access works in exactly one mode at a time.

- `APPLICATION_ACCESS_MODE=email_invite`
  - bootstrap admins grant access by email invite
  - requires SMTP
  - workspace-sharing invites by email are available
- `APPLICATION_ACCESS_MODE=trusted_email`
  - bootstrap admins add specific trusted emails in the UI
  - no SMTP is required
  - the trusted user can self-register with that exact email and gets their own workspace
  - workspace-sharing invites by email are disabled in this mode

### 5. Open local emails

Mailpit catches emails locally instead of sending them to the internet.

Open:

- `http://127.0.0.1:8025`

There you can open invitation emails and click links.

## Does production work

Yes, the production build can work, but only if you set the required environment correctly.

The critical production requirements are:

- real PostgreSQL
- HTTPS in front of the app
- stable `APP_BASE_URL`
- persistent `CERT_STORAGE_KEY`

SMTP is required only when `APPLICATION_ACCESS_MODE=email_invite`.

Without that, production is not correctly configured.

### Production assumptions

This app assumes:

- one deployed server process
- one PostgreSQL database
- reverse proxy or load balancer terminating TLS
- either SMTP-backed invite onboarding or trusted-email onboarding, depending on `APPLICATION_ACCESS_MODE`

### Production-critical env

| Variable | Required | Notes |
|---|---|---|
| `DATABASE_URL` | yes | PostgreSQL recommended |
| `APP_BASE_URL` | yes | Must be the public HTTPS URL of the app |
| `KSEF_ENVIRONMENT` | yes | Usually `production` or `test` |
| `CERT_STORAGE_KEY` | yes in production | Used to encrypt stored certs/keys in DB |
| `APPLICATION_ACCESS_MODE` | yes | `email_invite` or `trusted_email` |
| `SMTP_HOST` | yes when `APPLICATION_ACCESS_MODE=email_invite` | Real SMTP provider |
| `SMTP_PORT` | yes when `APPLICATION_ACCESS_MODE=email_invite` | Usually `587` |
| `SMTP_SECURITY` | yes when `APPLICATION_ACCESS_MODE=email_invite` | Usually `starttls` |
| `SMTP_AUTH` | yes when `APPLICATION_ACCESS_MODE=email_invite` | Usually `required` |
| `SMTP_USERNAME` | yes when auth required | SMTP login |
| `SMTP_PASSWORD` | yes when auth required | SMTP password |
| `SMTP_FROM_EMAIL` | yes when `APPLICATION_ACCESS_MODE=email_invite` | Sender email |
| `SMTP_FROM_NAME` | no | Display name |
| `ALLOWED_EMAILS` | yes | Bootstrap admins only |
| `SERVER_HOST` | no | Usually `0.0.0.0` |
| `SERVER_PORT` | no | Usually `3000` |

### Production certificate model

KsefPay stores NIP certificates per NIP account, not per user.

That means:

- a workspace can manage many NIP accounts
- each NIP account can have its own certificate/key
- certificate material is encrypted at rest in the database
- users without workspace access cannot reach that NIP

For production, keep in mind:

- `CERT_STORAGE_KEY` must be backed up
- if you lose `CERT_STORAGE_KEY`, you lose the ability to decrypt stored certs

### Production deployment with Docker Compose

Minimal path:

```sh
cp .env.example .env
```

Then edit `.env` for production values.

If you use SMTP-backed invites:

```env
DATABASE_URL=postgres://ksef:strong-password@postgres:5432/ksef
APP_BASE_URL=https://app.example.com
KSEF_ENVIRONMENT=production
CERT_STORAGE_KEY=replace-with-stable-secret
APPLICATION_ACCESS_MODE=email_invite

SMTP_HOST=smtp.example.com
SMTP_PORT=587
SMTP_SECURITY=starttls
SMTP_AUTH=required
SMTP_USERNAME=smtp-user
SMTP_PASSWORD=smtp-password
SMTP_FROM_EMAIL=noreply@example.com
SMTP_FROM_NAME=KSeF Pay

ALLOWED_EMAILS=owner@example.com
```

If you run without SMTP:

```env
DATABASE_URL=postgres://ksef:strong-password@postgres:5432/ksef
APP_BASE_URL=https://app.example.com
KSEF_ENVIRONMENT=production
CERT_STORAGE_KEY=replace-with-stable-secret
APPLICATION_ACCESS_MODE=trusted_email

ALLOWED_EMAILS=owner@example.com
```

Then:

```sh
make prod-up
```

Equivalent raw command:

```sh
docker compose -f docker-compose.prod.yml up --build
```

## Recommended production setup

For a proper production setup, use:

### 1. Reverse proxy with HTTPS

Example choices:

- Nginx
- Caddy
- Traefik

The app itself listens on HTTP, usually on port `3000`.
TLS should be terminated before it.

### 2. Real SMTP provider

Examples:

- Postmark
- Mailgun
- SendGrid
- Amazon SES

Do not use Mailpit in production.

### 3. Persistent PostgreSQL volume and backups

You need:

- regular DB backups
- restore procedure tested
- the same `CERT_STORAGE_KEY` available during restore

### 4. Production logging

Set a reasonable `RUST_LOG`, for example:

```env
RUST_LOG=info,ksef_core=info,ksef_server=info
```

### 5. Real bootstrap admins only

`ALLOWED_EMAILS` is no longer a list of all allowed users.
It should contain only the small set of bootstrap admins who are allowed to grant further access from the UI.

## Can you run production with one command

Yes, technically:

```sh
make prod-up
```

But only after you prepare the production `.env` correctly.

So the honest answer is:

- local dev: yes, one command
- production runtime: yes, one command to start
- production setup: no, not “one command”, because secrets, DNS, HTTPS, SMTP and backups must be configured first

## Tests

The current repo passes:

```sh
cargo check
cargo test
cargo test -- --ignored
```

That includes:

- unit tests
- SQLite integration tests
- PostgreSQL integration tests
- ignored tests available in this environment

## Development notes

### FA(3) generated bindings

This repo has a guard for generated FA(3) bindings.
If the build says bindings are stale, regenerate them with:

```sh
KSEF_FA3_SKIP_BINDINGS_CHECK=1 cargo run -p ksef-core --example generate_fa3_types
```

Then run normal commands again.

### Local SMTP

Local development uses Mailpit:

- SMTP: `127.0.0.1:1025`
- UI: `http://127.0.0.1:8025`

## Architecture

```text
ksef-core/       domain, ports, services, infra
ksef-server/     Axum server, HTML routes, templates
```

The project is structured as clean architecture:

- `domain`
- `ports`
- `services`
- `infra`
- `server`

## License

[MIT](LICENSE)
