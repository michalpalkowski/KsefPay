# KSeF Test Certificate — How To

## Requirements

KSeF test environment (`api-test.ksef.mf.gov.pl`) accepts self-signed
certificates, but the certificate **MUST** contain the NIP in the
`organizationIdentifier` field (OID 2.5.4.97) with a `VATPL-` prefix.

Example subject:
```
C = PL, O = ksef-paymoney, CN = KSeF Test, serialNumber = 5260250274, organizationIdentifier = VATPL-5260250274
```

Without `organizationIdentifier = VATPL-{NIP}`, KSeF returns:
```
exceptionCode: 21115, "Nieprawidłowy certyfikat."
```

## Generate Certificate

```sh
NIP=5260250274  # replace with your test NIP

openssl req -x509 -newkey rsa:2048 \
  -keyout .tmp/key.pem \
  -out .tmp/cert.pem \
  -days 365 -nodes \
  -subj "/C=PL/O=ksef-paymoney/CN=KSeF Test/serialNumber=$NIP/organizationIdentifier=VATPL-$NIP"
```

## Setup Test Subject on Sandbox

Before authenticating, the NIP must be registered as a test subject:

```sh
# 1. Create subject (one-time)
curl -X POST "https://api-test.ksef.mf.gov.pl/api/v2/testdata/subject" \
  -H "Content-Type: application/json" \
  -d "{\"subjectNip\":\"$NIP\",\"subjectType\":\"EnforcementAuthority\",\"description\":\"ksef-paymoney test\"}"

# 2. Grant permissions (may return 500 on sandbox — known issue)
# Permissions may already be inherited from the subject type.
```

## Run E2E Tests

```sh
KSEF_E2E_CERT_PEM=.tmp/cert.pem KSEF_E2E_KEY_PEM=.tmp/key.pem \
  cargo test -p ksef-core --test ksef_e2e -- --ignored --nocapture --test-threads=1
```

## Validated Flow (2026-04-13)

Full E2E passed on KSeF sandbox:
- Challenge → XAdES-BES auth → JWT → Open session → Send invoice → Close session
- Invoice received KSeF number: `20260413-EE-3BDD6E1000-B28577119B-6A`
- NIP used: `5260250274` (Ministerstwo Finansów — well-known test NIP with existing permissions)

## Production Certificates

For production, you need one of:
- **Qualified electronic signature** (podpis kwalifikowany) — hardware token with PESEL/NIP
- **KSeF certificate Type I** — issued by MCU (Modul Certyfikatow i Uprawnien), valid 2 years
- **Qualified seal** (pieczec kwalifikowana) — company-level, contains NIP

KSeF tokens (login.gov.pl based) are being **deprecated** — they work until 2026-12-31,
then only certificates will be accepted (from 2027-01-01).
