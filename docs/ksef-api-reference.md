# KSeF 2.0 API Technical Reference for Rust Integration

> This document covers the precise technical details needed to implement a Rust client
> for the Polish KSeF (Krajowy System e-Faktur) 2.0 API. It focuses on exact
> request/response schemas, encryption formats, XAdES signing, FA(3) XML structure,
> and test data endpoints -- material NOT covered in the existing high-level research.

---

## Table of Contents

1. [API Base URLs and Common Headers](#1-api-base-urls-and-common-headers)
2. [Authentication Flow -- Detailed Endpoint Schemas](#2-authentication-flow----detailed-endpoint-schemas)
3. [Session Management Endpoints](#3-session-management-endpoints)
4. [Invoice Submission and Retrieval](#4-invoice-submission-and-retrieval)
5. [Security / Public Key Endpoints](#5-security--public-key-endpoints)
6. [Encryption Specification](#6-encryption-specification)
7. [XAdES AuthTokenRequest Signing](#7-xades-authtokenrequest-signing)
8. [FA(3) XML Schema Reference](#8-fa3-xml-schema-reference)
9. [Test Data Endpoints](#9-test-data-endpoints)
10. [Error Handling](#10-error-handling)
11. [Rate Limits and Constraints](#11-rate-limits-and-constraints)

---

## 1. API Base URLs and Common Headers

### Base URLs

| Environment | Base URL |
|---|---|
| Test (TE) | `https://api-test.ksef.mf.gov.pl/api/v2` |
| Demo (TR) | `https://api-demo.ksef.mf.gov.pl/api/v2` |
| Production (PRD) | `https://api.ksef.mf.gov.pl/api/v2` |

### Common Request Headers

```
Content-Type: application/json
Accept: application/json
```

After authentication, all requests requiring authorization use:

```
Authorization: Bearer <accessToken>
```

The accessToken is a JWT with ~15 minute TTL. The API returns HTTP 401 when it
expires, at which point you must use the refresh token flow.

### Common Response Envelope

Most responses follow this pattern:

```json
{
  "timestamp": "2026-04-13T10:30:00.000Z",
  "referenceNumber": "...",
  ...endpoint-specific fields...
}
```

Error responses:

```json
{
  "exception": {
    "serviceCtx": "srvTESTKSEF",
    "serviceCode": "20260413-EX-...",
    "serviceName": "online.auth.challenge",
    "timestamp": "2026-04-13T10:30:00.000Z",
    "referenceNumber": null,
    "exceptionDetailList": [
      {
        "exceptionCode": 21001,
        "exceptionDescription": "Podmiot o podanym identyfikatorze nie istnieje"
      }
    ]
  }
}
```

---

## 2. Authentication Flow -- Detailed Endpoint Schemas

The full authentication flow is:

```
POST /auth/challenge
    --> receive challenge token + timestamp
         |
         v
Build AuthTokenRequest XML with challenge + NIP
    --> sign with XAdES (Enveloped or Enveloping)
         |
         v
POST /auth/xades-signature  (or /auth/ksef-token for token auth)
    --> receive referenceNumber
         |
         v
GET /auth/{referenceNumber}  (poll until status != PENDING)
    --> status: AUTHORIZED | REJECTED | EXPIRED
         |
         v
POST /auth/token/redeem
    --> receive accessToken (JWT) + refreshToken
         |
         v
Use "Authorization: Bearer <accessToken>" for all subsequent requests
```

### 2.1 POST /auth/challenge

Requests a cryptographic challenge that must be signed to prove identity.

**Request Body:**

```json
{
  "contextIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  }
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `contextIdentifier.type` | string (enum) | Yes | `"onip"` -- identifies subject by NIP |
| `contextIdentifier.identifier` | string | Yes | 10-digit NIP number (no dashes) |

The `type` field can also be `"pesel"` for person-based auth, but for business
invoice operations, `"onip"` is standard.

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:30:00.000Z",
  "challenge": "20260413-CR-XXXXXXXXXX-YYYYYYYY-ZZ",
  "challengeExpirationTimestamp": "2026-04-13T10:40:00.000Z"
}
```

| Field | Type | Description |
|---|---|---|
| `timestamp` | string (ISO 8601) | Server timestamp |
| `challenge` | string | The challenge token (opaque string, ~40 chars). Must be embedded in AuthTokenRequest XML |
| `challengeExpirationTimestamp` | string (ISO 8601) | Challenge validity -- typically 10 minutes from creation |

**Rust mapping:**

```rust
#[derive(Serialize)]
struct ChallengeRequest {
    #[serde(rename = "contextIdentifier")]
    context_identifier: ContextIdentifier,
}

#[derive(Serialize)]
struct ContextIdentifier {
    #[serde(rename = "type")]
    identifier_type: String,  // "onip"
    identifier: String,       // NIP
}

#[derive(Deserialize)]
struct ChallengeResponse {
    timestamp: String,
    challenge: String,
    #[serde(rename = "challengeExpirationTimestamp")]
    challenge_expiration_timestamp: String,
}
```

### 2.2 POST /auth/xades-signature

Submits the XAdES-signed AuthTokenRequest to initiate authentication.

**Request Body:**

```json
{
  "authData": "PD94bWwgdmVyc2lvbj0iMS4wIi...base64-encoded-signed-XML..."
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `authData` | string (base64) | Yes | Base64-encoded XAdES-signed AuthTokenRequest XML document |

The `authData` is the **complete** signed XML document (the AuthTokenRequest with
the XAdES signature embedded), encoded as base64 (standard, not URL-safe).

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:30:15.000Z",
  "referenceNumber": "20260413-SE-XXXXXXXXXX-YYYYYYYY-ZZ",
  "authenticationIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `timestamp` | string (ISO 8601) | Server timestamp |
| `referenceNumber` | string | Reference for polling auth status (use in GET /auth/{ref}) |
| `authenticationIdentifier` | object | Echo of the subject being authenticated |

### 2.3 GET /auth/{referenceNumber}

Polls the authentication status. Call this in a loop with ~1-2 second delays.

**Path Parameters:**

| Param | Type | Description |
|---|---|---|
| `referenceNumber` | string | The reference from POST /auth/xades-signature |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:30:20.000Z",
  "authenticationStatus": {
    "status": "AUTHORIZED",
    "referenceNumber": "20260413-SE-XXXXXXXXXX-YYYYYYYY-ZZ",
    "authenticationIdentifier": {
      "type": "onip",
      "identifier": "5213609827"
    }
  }
}
```

**Status Values (enum):**

| Status | Meaning | Action |
|---|---|---|
| `PENDING` | Authentication still being processed | Continue polling (wait 1-2s) |
| `AUTHORIZED` | Authentication successful | Proceed to POST /auth/token/redeem |
| `REJECTED` | Authentication failed (bad signature, wrong NIP, etc.) | Abort, check error details |
| `EXPIRED` | Challenge or auth request timed out | Start over from POST /auth/challenge |

When `status` is `REJECTED`, additional error details may appear in an
`exceptionDetailList` field within the response.

### 2.4 POST /auth/token/redeem

Exchanges the authorized reference for JWT tokens.

**Request Body:**

```json
{
  "referenceNumber": "20260413-SE-XXXXXXXXXX-YYYYYYYY-ZZ"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `referenceNumber` | string | Yes | The reference from the AUTHORIZED auth status |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:30:25.000Z",
  "accessToken": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4...",
  "accessTokenExpiresIn": 900,
  "refreshTokenExpiresIn": 604800,
  "contextIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `accessToken` | string | JWT Bearer token for API calls (~15 min TTL) |
| `refreshToken` | string | Opaque token for refreshing the access token (up to 7 days) |
| `accessTokenExpiresIn` | integer | Access token TTL in seconds (typically 900 = 15 min) |
| `refreshTokenExpiresIn` | integer | Refresh token TTL in seconds (typically 604800 = 7 days) |
| `contextIdentifier` | object | The authenticated subject |

### 2.5 POST /auth/token/refresh

Refreshes an expired access token using the refresh token.

**Request Body:**

```json
{
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4..."
}
```

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:45:25.000Z",
  "accessToken": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...(new)...",
  "refreshToken": "bmV3IHJlZnJlc2ggdG9rZW4...",
  "accessTokenExpiresIn": 900,
  "refreshTokenExpiresIn": 604800
}
```

A new refresh token is issued with each refresh. The old refresh token is
invalidated. If the refresh token itself has expired (after 7 days), the full
auth flow (challenge -> sign -> redeem) must be repeated.

---

## 3. Session Management Endpoints

### 3.1 POST /sessions/online

Opens an interactive session for sending invoices. Sessions are valid for 12 hours.

**Headers:**

```
Authorization: Bearer <accessToken>
Content-Type: application/json
```

**Request Body:**

```json
{
  "contextIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  },
  "formCode": {
    "systemCode": "FA (3)",
    "schemaVersion": "1-0E",
    "targetNamespace": "http://crd.gov.pl/wzor/2025/06/25/13775/",
    "value": "FA"
  }
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `contextIdentifier.type` | string | Yes | `"onip"` |
| `contextIdentifier.identifier` | string | Yes | NIP of the session owner |
| `formCode.systemCode` | string | Yes | `"FA (3)"` for the current invoice schema |
| `formCode.schemaVersion` | string | Yes | `"1-0E"` (version of FA(3) XSD) |
| `formCode.targetNamespace` | string | Yes | `"http://crd.gov.pl/wzor/2025/06/25/13775/"` |
| `formCode.value` | string | Yes | `"FA"` |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:31:00.000Z",
  "referenceNumber": "20260413-SO-XXXXXXXXXX-YYYYYYYY-ZZ",
  "sessionStatus": {
    "processingCode": 100,
    "processingDescription": "Sesja otwarta"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `referenceNumber` | string | Session reference -- use this for all in-session operations |
| `sessionStatus.processingCode` | integer | 100 = session opened successfully |
| `sessionStatus.processingDescription` | string | Human-readable status |

**Processing codes for session status:**

| Code | Meaning |
|---|---|
| 100 | Session active / opened |
| 200 | Session processing (invoices being processed) |
| 300 | Session terminated normally |
| 310 | Session terminated due to inactivity |
| 315 | Session expired (12h TTL) |
| 400 | Session error |

### 3.2 POST /sessions/online/{referenceNumber}/invoices

Sends an encrypted invoice within an active session.

**Path Parameters:**

| Param | Type | Description |
|---|---|---|
| `referenceNumber` | string | Session reference from POST /sessions/online |

**Headers:**

```
Authorization: Bearer <accessToken>
Content-Type: application/json
```

**Request Body:**

```json
{
  "invoicePayload": {
    "type": "encrypted",
    "encryptedInvoiceHash": {
      "hashSHA": {
        "algorithm": "SHA-256",
        "encoding": "Base64",
        "value": "<base64-SHA256-of-plaintext-invoice-XML>"
      },
      "fileSize": 2048
    },
    "encryptedInvoiceBody": "<base64-encoded-encrypted-package>"
  }
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `invoicePayload.type` | string | Yes | Always `"encrypted"` |
| `encryptedInvoiceHash.hashSHA.algorithm` | string | Yes | `"SHA-256"` |
| `encryptedInvoiceHash.hashSHA.encoding` | string | Yes | `"Base64"` |
| `encryptedInvoiceHash.hashSHA.value` | string | Yes | Base64-encoded SHA-256 hash of the **plaintext** (unencrypted) invoice XML bytes |
| `encryptedInvoiceHash.fileSize` | integer | Yes | Size in bytes of the **plaintext** invoice XML |
| `invoicePayload.encryptedInvoiceBody` | string | Yes | Base64-encoded encrypted package (see Section 6 for format) |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:35:00.000Z",
  "referenceNumber": "20260413-SO-XXXXXXXXXX-YYYYYYYY-ZZ",
  "processingCode": 200,
  "processingDescription": "Faktura przyjeta do przetwarzania",
  "elementReferenceNumber": "20260413-EL-XXXXXXXXXX-YYYYYYYY-ZZ",
  "invoiceStatus": {
    "invoiceNumber": "FV/2026/04/001"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `elementReferenceNumber` | string | Unique reference for this specific invoice submission |
| `processingCode` | integer | 200 = accepted for processing |
| `invoiceStatus.invoiceNumber` | string | The original invoice number from the XML |

After submission, the invoice goes through processing. The KSeF number
(e.g., `1234567890-20260413-XXXXXXXXXX-XX`) is assigned asynchronously.
You must poll or query to get the final KSeF number.

### 3.3 POST /sessions/online/{referenceNumber}/close

Closes an active session. Returns session summary and triggers UPO generation.

**Request Body:** Empty JSON object `{}` or no body.

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T11:00:00.000Z",
  "referenceNumber": "20260413-SO-XXXXXXXXXX-YYYYYYYY-ZZ",
  "sessionStatus": {
    "processingCode": 300,
    "processingDescription": "Sesja zakonczona"
  },
  "numberOfInvoices": 5,
  "upoReferenceNumber": "20260413-UP-XXXXXXXXXX-YYYYYYYY-ZZ"
}
```

| Field | Type | Description |
|---|---|---|
| `sessionStatus.processingCode` | integer | 300 = session terminated normally |
| `numberOfInvoices` | integer | Total invoices processed in this session |
| `upoReferenceNumber` | string | Reference to retrieve the UPO document |

### 3.4 GET /sessions/{referenceNumber}/upo

Retrieves the UPO (Urzedowe Potwierdzenie Odbioru -- Official Confirmation of
Receipt) for a closed session.

**Headers:**

```
Authorization: Bearer <accessToken>
Accept: application/json   (for JSON metadata)
Accept: application/pdf    (for PDF document)
Accept: application/xml    (for XML UPO)
```

**Response (200 OK) -- JSON:**

```json
{
  "timestamp": "2026-04-13T11:05:00.000Z",
  "referenceNumber": "20260413-UP-XXXXXXXXXX-YYYYYYYY-ZZ",
  "upo": "<base64-encoded-UPO-XML>",
  "processingCode": 200,
  "processingDescription": "UPO wygenerowane"
}
```

The UPO may not be immediately available after session close. If `processingCode`
is not 200, poll again after a few seconds. The UPO contains a signed confirmation
of all invoices submitted in the session.

**Processing codes for UPO:**

| Code | Meaning |
|---|---|
| 100 | UPO generation in progress |
| 200 | UPO ready |
| 400 | UPO generation error |

---

## 4. Invoice Submission and Retrieval

### 4.1 GET /invoices/ksef/{ksefNumber}

Retrieves a specific invoice by its KSeF-assigned number.

**Path Parameters:**

| Param | Type | Description |
|---|---|---|
| `ksefNumber` | string | The KSeF number, e.g. `1234567890-20260413-XXXXXXXXXX-XX` |

**Headers:**

```
Authorization: Bearer <accessToken>
Accept: application/json   (returns metadata + base64 XML)
Accept: application/xml    (returns raw invoice XML)
```

**Response (200 OK) -- JSON:**

```json
{
  "timestamp": "2026-04-13T12:00:00.000Z",
  "invoiceStatus": "APPROVED",
  "ksefReferenceNumber": "1234567890-20260413-XXXXXXXXXX-XX",
  "invoiceDetails": {
    "invoiceOryginalNumber": "FV/2026/04/001",
    "subjectBy": {
      "issuedByIdentifier": {
        "type": "onip",
        "identifier": "5213609827"
      },
      "issuedByName": {
        "tradeName": "Firma Testowa Sp. z o.o."
      }
    },
    "subjectTo": {
      "issuedToIdentifier": {
        "type": "onip",
        "identifier": "1234567890"
      },
      "issuedToName": {
        "tradeName": "Odbiorca Testowy S.A."
      }
    },
    "invoicingDate": "2026-04-13",
    "acquisitionTimestamp": "2026-04-13T10:35:15.123Z",
    "net": "10000.00",
    "vat": "2300.00",
    "gross": "12300.00",
    "currency": "PLN",
    "schemaVersion": "FA (3)"
  },
  "invoiceBody": "<base64-encoded-invoice-XML>"
}
```

### 4.2 GET /invoices/query/metadata

Searches for invoices matching given criteria. Used to discover incoming
invoices (purchase invoices sent to your NIP by other entities).

**Query Parameters:**

| Param | Type | Required | Description |
|---|---|---|---|
| `subjectType` | string | Yes | `"subject1"` (seller/sent by me) or `"subject2"` (buyer/sent to me) |
| `dateFrom` | string (ISO 8601) | Yes | Start of date range (acquisition timestamp) |
| `dateTo` | string (ISO 8601) | Yes | End of date range |
| `pageSize` | integer | No | Results per page (default 10, max 100) |
| `pageOffset` | integer | No | Page offset for pagination (0-based) |
| `invoiceNumber` | string | No | Filter by original invoice number (partial match) |
| `ksefReferenceNumber` | string | No | Filter by KSeF reference number |
| `nipSender` | string | No | Filter by sender NIP |
| `nipRecipient` | string | No | Filter by recipient NIP |
| `amountFrom` | number | No | Minimum gross amount |
| `amountTo` | number | No | Maximum gross amount |
| `currencyCode` | string | No | Currency code filter (e.g., `"PLN"`) |

**Headers:**

```
Authorization: Bearer <accessToken>
```

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T12:00:00.000Z",
  "invoiceMetadataList": [
    {
      "ksefReferenceNumber": "1234567890-20260413-XXXXXXXXXX-XX",
      "invoiceOryginalNumber": "FV/2026/04/001",
      "invoicingDate": "2026-04-13",
      "acquisitionTimestamp": "2026-04-13T10:35:15.123Z",
      "subjectBy": {
        "issuedByIdentifier": { "type": "onip", "identifier": "5213609827" },
        "issuedByName": { "tradeName": "Firma Testowa Sp. z o.o." }
      },
      "subjectTo": {
        "issuedToIdentifier": { "type": "onip", "identifier": "1234567890" },
        "issuedToName": { "tradeName": "Odbiorca Testowy S.A." }
      },
      "net": "10000.00",
      "vat": "2300.00",
      "gross": "12300.00",
      "currency": "PLN",
      "invoiceStatus": "APPROVED",
      "schemaVersion": "FA (3)"
    }
  ],
  "numberOfElements": 1,
  "pageSize": 10,
  "pageOffset": 0
}
```

---

## 5. Security / Public Key Endpoints

### 5.1 GET /security/public-key-certificates

Returns the RSA public keys used by KSeF for invoice encryption. You encrypt
the AES symmetric key with one of these public keys.

**No authorization required.**

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:00:00.000Z",
  "publicKeyPemList": [
    {
      "kid": "ksef-test-key-2026-001",
      "publicKeyPem": "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...\n-----END PUBLIC KEY-----",
      "algorithm": "RSA",
      "keyLength": 2048,
      "validFrom": "2025-09-01T00:00:00.000Z",
      "validTo": "2027-09-01T00:00:00.000Z",
      "isActive": true
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `kid` | string | Key identifier -- include in the encrypted package header |
| `publicKeyPem` | string | PEM-encoded RSA public key |
| `algorithm` | string | `"RSA"` |
| `keyLength` | integer | Key size in bits (2048 or 4096) |
| `validFrom` | string | Key validity start |
| `validTo` | string | Key validity end |
| `isActive` | boolean | Whether this key is currently accepted |

**Implementation note:** Always use the key where `isActive == true`. Cache the
key but refresh periodically (e.g., daily) as KSeF rotates keys. There may be
multiple active keys during rotation periods.

---

## 6. Encryption Specification

Every invoice sent to KSeF **must** be encrypted. The encryption uses a hybrid
scheme: AES-256-CBC for the invoice data, and RSA-OAEP for the AES key.

### 6.1 Encryption Algorithm Details

| Component | Algorithm | Details |
|---|---|---|
| Symmetric encryption | AES-256-CBC | 256-bit key, PKCS#7 padding |
| IV (Initialization Vector) | Random | 16 bytes (128 bits), cryptographically random |
| Key encryption | RSA-OAEP | SHA-256 hash, MGF1 with SHA-256 |
| Hash of plaintext | SHA-256 | Hash of the raw XML bytes before encryption |

### 6.2 Encryption Procedure (Step by Step)

```
1. Fetch KSeF public key from GET /security/public-key-certificates
   (use the active key)

2. Prepare the plaintext invoice XML (FA(3) schema, UTF-8 encoded bytes)

3. Compute SHA-256 hash of the plaintext XML bytes
   --> used in the invoicePayload.encryptedInvoiceHash.hashSHA.value field

4. Generate a random 256-bit (32-byte) AES key

5. Generate a random 128-bit (16-byte) IV

6. Encrypt the plaintext XML with AES-256-CBC using the key and IV
   (with PKCS#7 padding)

7. Encrypt the AES key with RSA-OAEP using the KSeF public key
   (SHA-256, MGF1-SHA256)

8. Assemble the encrypted package (see 6.3)

9. Base64-encode the entire package

10. Send as invoicePayload.encryptedInvoiceBody
```

### 6.3 Encrypted Package Binary Format

The encrypted package sent in `encryptedInvoiceBody` (after base64 encoding)
has the following binary structure:

```
+--------------------------------------------------+
| Encrypted AES Key (RSA-OAEP)                     |
| Length: equal to RSA key size (256 bytes for      |
|         2048-bit key, 512 bytes for 4096-bit)     |
+--------------------------------------------------+
| IV (Initialization Vector)                        |
| Length: 16 bytes (fixed)                          |
+--------------------------------------------------+
| AES-256-CBC Encrypted Invoice Data                |
| Length: variable (PKCS#7 padded)                  |
+--------------------------------------------------+
```

**Concatenation order:** `encrypted_aes_key || iv || encrypted_data`

The server knows the RSA key length (from the key ID), so it can parse:
- First N bytes = encrypted AES key (N = RSA key size in bytes)
- Next 16 bytes = IV
- Remaining bytes = encrypted invoice data

### 6.4 Rust Pseudocode

```rust
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use rsa::{Oaep, RsaPublicKey};
use sha2::Sha256;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;

fn encrypt_invoice(
    plaintext_xml: &[u8],
    ksef_public_key: &RsaPublicKey,
) -> Result<(String, String, usize), CryptoError> {
    // 1. Hash the plaintext
    let hash = Sha256::digest(plaintext_xml);
    let hash_b64 = base64::engine::general_purpose::STANDARD.encode(hash);

    // 2. Generate random AES key and IV
    let mut aes_key = [0u8; 32];
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut aes_key);
    OsRng.fill_bytes(&mut iv);

    // 3. Encrypt invoice with AES-256-CBC
    let encrypted_data = Aes256CbcEnc::new(&aes_key.into(), &iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext_xml);

    // 4. Encrypt AES key with RSA-OAEP
    let padding = Oaep::new_with_mgf_hash::<Sha256, Sha256>();
    let encrypted_key = ksef_public_key
        .encrypt(&mut OsRng, padding, &aes_key)?;

    // 5. Assemble package: encrypted_key || iv || encrypted_data
    let mut package = Vec::with_capacity(
        encrypted_key.len() + iv.len() + encrypted_data.len()
    );
    package.extend_from_slice(&encrypted_key);
    package.extend_from_slice(&iv);
    package.extend_from_slice(&encrypted_data);

    // 6. Base64 encode
    let encrypted_body = base64::engine::general_purpose::STANDARD.encode(&package);

    Ok((encrypted_body, hash_b64, plaintext_xml.len()))
}
```

### 6.5 Important Notes

- The `fileSize` in the request is the size of the **plaintext** XML, not the
  encrypted package.
- The `hashSHA.value` is the SHA-256 of the **plaintext** XML, not the encrypted
  data. KSeF uses this to verify integrity after decryption.
- The base64 encoding is **standard** base64 (with `+`, `/`, and `=` padding),
  NOT base64url.
- Maximum plaintext invoice size: 1 MB without attachments, 3 MB with attachments.

---

## 7. XAdES AuthTokenRequest Signing

### 7.1 AuthTokenRequest XML Structure

Before signing, you must construct an `AuthTokenRequest` XML document that
contains the challenge from `/auth/challenge` and your NIP.

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<AuthTokenRequest xmlns="http://ksef.mf.gov.pl/schema/auth/request/2021/10/01/0001">
    <ContextIdentifier>
        <Type>onip</Type>
        <Identifier>5213609827</Identifier>
    </ContextIdentifier>
    <Challenge>20260413-CR-XXXXXXXXXX-YYYYYYYY-ZZ</Challenge>
</AuthTokenRequest>
```

**XML Namespace:** `http://ksef.mf.gov.pl/schema/auth/request/2021/10/01/0001`

| Element | Required | Description |
|---|---|---|
| `ContextIdentifier/Type` | Yes | `"onip"` for NIP-based auth |
| `ContextIdentifier/Identifier` | Yes | 10-digit NIP |
| `Challenge` | Yes | The challenge string from POST /auth/challenge (copy exactly) |

### 7.2 XAdES Signature Requirements

| Requirement | Value |
|---|---|
| **XAdES Profile** | XAdES-BES (Basic Electronic Signature) |
| **Signature Format** | Enveloped or Enveloping (NOT Detached) |
| **Canonicalization** | Exclusive XML Canonicalization (exc-c14n) `http://www.w3.org/2001/10/xml-exc-c14n#` |
| **Digest Algorithm** | SHA-256 `http://www.w3.org/2001/04/xmlenc#sha256` |
| **Signature Algorithm** | RSA-SHA256 `http://www.w3.org/2001/04/xmldsig-more#rsa-sha256` |
| **Certificate** | Test env: self-signed X.509; Production: qualified electronic signature or KSeF certificate (Type I) |
| **KeyInfo** | Must include the X.509 certificate in the signature |

### 7.3 XAdES-BES Enveloped Signature Structure

After signing, the document looks like this (simplified):

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<AuthTokenRequest xmlns="http://ksef.mf.gov.pl/schema/auth/request/2021/10/01/0001">
    <ContextIdentifier>
        <Type>onip</Type>
        <Identifier>5213609827</Identifier>
    </ContextIdentifier>
    <Challenge>20260413-CR-XXXXXXXXXX-YYYYYYYY-ZZ</Challenge>
    <ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"
                  Id="Signature-1">
        <ds:SignedInfo>
            <ds:CanonicalizationMethod
                Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/>
            <ds:SignatureMethod
                Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/>
            <ds:Reference URI="">
                <ds:Transforms>
                    <ds:Transform
                        Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/>
                    <ds:Transform
                        Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/>
                </ds:Transforms>
                <ds:DigestMethod
                    Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>
                <ds:DigestValue>...</ds:DigestValue>
            </ds:Reference>
            <ds:Reference
                URI="#SignedProperties-1"
                Type="http://uri.etsi.org/01903#SignedProperties">
                <ds:DigestMethod
                    Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>
                <ds:DigestValue>...</ds:DigestValue>
            </ds:Reference>
        </ds:SignedInfo>
        <ds:SignatureValue>...</ds:SignatureValue>
        <ds:KeyInfo>
            <ds:X509Data>
                <ds:X509Certificate>MIID...base64-cert...</ds:X509Certificate>
            </ds:X509Data>
        </ds:KeyInfo>
        <ds:Object>
            <xades:QualifyingProperties
                xmlns:xades="http://uri.etsi.org/01903/v1.3.2#"
                Target="#Signature-1">
                <xades:SignedProperties Id="SignedProperties-1">
                    <xades:SignedSignatureProperties>
                        <xades:SigningTime>2026-04-13T10:30:10Z</xades:SigningTime>
                        <xades:SigningCertificateV2>
                            <xades:Cert>
                                <xades:CertDigest>
                                    <ds:DigestMethod
                                        Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>
                                    <ds:DigestValue>...cert-hash...</ds:DigestValue>
                                </xades:CertDigest>
                                <xades:IssuerSerial>
                                    <ds:X509IssuerName>CN=Test,O=Test</ds:X509IssuerName>
                                    <ds:X509SerialNumber>123456789</ds:X509SerialNumber>
                                </xades:IssuerSerial>
                            </xades:Cert>
                        </xades:SigningCertificateV2>
                    </xades:SignedSignatureProperties>
                </xades:SignedProperties>
            </xades:QualifyingProperties>
        </ds:Object>
    </ds:Signature>
</AuthTokenRequest>
```

### 7.4 Key XAdES Elements Explained

| Element | Purpose |
|---|---|
| `SigningTime` | ISO 8601 timestamp of when the signature was created |
| `SigningCertificateV2` | Contains hash of the signing certificate (binds cert to signature) |
| `Reference URI=""` | References the entire document (enveloped signature transform excludes the Signature element itself) |
| `Reference URI="#SignedProperties-1"` | Signs the XAdES properties to prevent tampering |
| `X509Certificate` | The full signing certificate in base64 DER encoding |

### 7.5 Self-Signed Certificate for Test Environment

For the test environment, generate a self-signed certificate:

```
# Generate RSA key pair and self-signed certificate (valid 365 days)
openssl req -x509 -newkey rsa:2048 -keyout test-key.pem -out test-cert.pem \
  -days 365 -nodes -subj "/CN=KSeF Test/O=Test Company/C=PL"
```

In Rust, use the `openssl` crate to load the key and certificate, or generate
them programmatically.

### 7.6 Rust Implementation Strategy for XAdES

XAdES signing is the most complex part of the KSeF integration. Options:

1. **`openssl` crate + manual XML construction**: Build the XML-DSig and XAdES
   structures manually, compute digests with OpenSSL, sign with RSA-SHA256.
   This is the most control but requires careful XML canonicalization.

2. **Shell out to `xmlsec1`**: The `xmlsec1` command-line tool supports XAdES.
   Simpler but adds a system dependency.
   ```
   xmlsec1 --sign --pkcs12 cert.p12 --pwd password \
     --output signed.xml template.xml
   ```

3. **Use the `xmlsec` Rust crate**: Bindings to the xmlsec C library. Less
   mature but keeps things in-process.

**Recommended approach for MVP**: Manual XML construction with the `openssl`
crate. The AuthTokenRequest is a very simple document, so the XAdES signing
is manageable. Steps:

```rust
// Pseudocode for XAdES-BES Enveloped signing
fn sign_auth_request(xml: &str, cert: &X509, key: &PKey<Private>) -> String {
    // 1. Canonicalize the XML (exc-c14n)
    // 2. Compute SHA-256 digest of canonicalized XML
    // 3. Build SignedInfo XML with Reference to "" (enveloped)
    //    and Reference to "#SignedProperties"
    // 4. Build QualifyingProperties with SigningTime and SigningCertificateV2
    // 5. Compute SHA-256 digest of canonicalized SignedProperties
    // 6. Canonicalize SignedInfo
    // 7. Sign canonicalized SignedInfo with RSA-SHA256
    // 8. Insert ds:Signature element into the AuthTokenRequest
    // 9. Return the complete signed XML
}
```

---

## 8. FA(3) XML Schema Reference

### 8.1 Schema Identification

| Property | Value |
|---|---|
| Schema name | FA (3) |
| Version | 1-0E |
| Root element | `Faktura` |
| Namespace | `http://crd.gov.pl/wzor/2025/06/25/13775/` |
| XSD location | `http://crd.gov.pl/wzor/2025/06/25/13775/` |
| Download | `https://ksef.podatki.gov.pl/media/ukrllh1e/schemat_fa_vat-3-_v1-0.xsd` |

### 8.2 Root Element and Namespaces

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <Naglowek>...</Naglowek>
    <Podmiot1>...</Podmiot1>
    <Podmiot2>...</Podmiot2>
    <Fa>...</Fa>
    <!-- Optional elements: Podmiot3, PodmiotUpowazniony, Stopka, Zalacznik -->
</Faktura>
```

### 8.3 Document Sections Overview

| Section | Required | Max Occurrences | Description |
|---|---|---|---|
| `Naglowek` | Yes | 1 | Header: creation timestamp, system info, schema version |
| `Podmiot1` | Yes | 1 | Seller (issuer) identification |
| `Podmiot2` | Yes | 1 | Buyer (recipient) identification |
| `Fa` | Yes | 1 | Invoice core: type, dates, amounts, line items, payment |
| `Podmiot3` | No | 100 | Additional subjects (intermediaries, etc.) |
| `PodmiotUpowazniony` | Conditional | 1 | Authorized third party issuing on behalf |
| `Stopka` | No | 1 | Footer: additional notes |
| `Zalacznik` | No | unbounded | Structured attachments (new in FA(3)) |

### 8.4 Minimal Invoice Example (Product Sale, 23% VAT)

This is a complete, minimal, valid FA(3) invoice for a simple product sale:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2025/06/25/13775/"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">

    <!-- HEADER -->
    <Naglowek>
        <KodFormularza kodSystemowy="FA (3)"
                       wersjaSchemy="1-0E">FA</KodFormularza>
        <WariantFormularza>3</WariantFormularza>
        <DataWytworzeniaFa>2026-04-13T10:00:00Z</DataWytworzeniaFa>
        <SystemInfo>KSeF-PayMoney-Rust/0.1.0</SystemInfo>
    </Naglowek>

    <!-- SELLER -->
    <Podmiot1>
        <DaneIdentyfikacyjne>
            <NIP>5213609827</NIP>
            <Nazwa>Firma Testowa Sp. z o.o.</Nazwa>
        </DaneIdentyfikacyjne>
        <Adres>
            <KodKraju>PL</KodKraju>
            <AdresL1>ul. Testowa 1</AdresL1>
            <AdresL2>00-001 Warszawa</AdresL2>
        </Adres>
    </Podmiot1>

    <!-- BUYER -->
    <Podmiot2>
        <DaneIdentyfikacyjne>
            <NIP>1234567890</NIP>
            <Nazwa>Odbiorca Testowy S.A.</Nazwa>
        </DaneIdentyfikacyjne>
        <Adres>
            <KodKraju>PL</KodKraju>
            <AdresL1>ul. Przykladowa 42</AdresL1>
            <AdresL2>00-002 Warszawa</AdresL2>
        </Adres>
    </Podmiot2>

    <!-- INVOICE CORE -->
    <Fa>
        <!-- Invoice type: VAT = standard invoice -->
        <KodWaluty>PLN</KodWaluty>
        <P_1>2026-04-13</P_1>       <!-- Invoice issue date -->
        <P_2>FV/2026/04/001</P_2>    <!-- Invoice number -->
        <P_6>2026-04-13</P_6>        <!-- Date of sale / service delivery -->

        <!-- P_13 group: VAT rate summary lines -->
        <!-- 23% VAT rate summary -->
        <P_13_1>10000.00</P_13_1>    <!-- Net amount at 23% -->
        <P_14_1>2300.00</P_14_1>     <!-- VAT amount at 23% -->

        <P_15>12300.00</P_15>        <!-- Total gross amount -->

        <!-- Invoice line items -->
        <FaWiersz>
            <NrWierszaFa>1</NrWierszaFa>
            <P_7>Usluga programistyczna</P_7>  <!-- Product/service name (max 512 chars) -->
            <P_8A>szt.</P_8A>                   <!-- Unit of measure -->
            <P_8B>1</P_8B>                      <!-- Quantity -->
            <P_9A>10000.00</P_9A>               <!-- Unit net price -->
            <P_11>10000.00</P_11>               <!-- Line net amount -->
            <P_11A>10000.00</P_11A>             <!-- Line net amount (redundant but required in some cases) -->
            <P_12>23</P_12>                     <!-- VAT rate (23, 8, 5, 0, "zw", "oo", "np") -->
        </FaWiersz>

        <!-- Payment information -->
        <Platnosc>
            <TerminPlatnosci>
                <Termin>2026-04-27</Termin>     <!-- Payment due date -->
            </TerminPlatnosci>
            <FormaPlatnosci>6</FormaPlatnosci>   <!-- 6 = bank transfer -->
            <RachunekBankowy>
                <NrRB>PL61109010140000071219812874</NrRB>
            </RachunekBankowy>
        </Platnosc>
    </Fa>

</Faktura>
```

### 8.5 Key Field Reference (Fa Section)

| Field | XML Element | Type | Required | Description |
|---|---|---|---|---|
| Currency | `KodWaluty` | string | Yes | ISO 4217 currency code (e.g., `PLN`, `EUR`) |
| Issue date | `P_1` | date | Yes | Invoice issue date (YYYY-MM-DD) |
| Invoice number | `P_2` | string | Yes | Sequential invoice number (your system's numbering) |
| Sale date | `P_6` | date | Yes* | Date of sale or service delivery (*or P_6 range) |
| Net at 23% | `P_13_1` | decimal | Conditional | Total net amount at 23% VAT rate |
| VAT at 23% | `P_14_1` | decimal | Conditional | Total VAT amount at 23% |
| Net at 8% | `P_13_2` | decimal | Conditional | Total net amount at 8% VAT rate |
| VAT at 8% | `P_14_2` | decimal | Conditional | Total VAT amount at 8% |
| Net at 5% | `P_13_3` | decimal | Conditional | Total net amount at 5% VAT rate |
| VAT at 5% | `P_14_3` | decimal | Conditional | Total VAT amount at 5% |
| Net at 0% | `P_13_6_1` | decimal | Conditional | Total net at 0% (domestic) |
| Net exempt | `P_13_7` | decimal | Conditional | Total net amount VAT-exempt ("zw") |
| Gross total | `P_15` | decimal | Yes | Total gross amount (sum of all net + all VAT) |

### 8.6 VAT Rate Values (P_12)

| Value | Meaning |
|---|---|
| `23` | Standard rate 23% |
| `22` | Legacy 22% (still valid for old contracts) |
| `8` | Reduced rate 8% |
| `7` | Legacy 7% |
| `5` | Reduced rate 5% |
| `4` | Special 4% rate |
| `3` | Special 3% rate |
| `0` | Zero rate 0% |
| `zw` | VAT-exempt (zwolniony) |
| `oo` | Reverse charge (odwrotne obciazenie) |
| `np` | Not subject to VAT (nie podlega) |

### 8.7 Invoice Line Items (FaWiersz)

Each line item must contain:

| Field | Element | Type | Required | Description |
|---|---|---|---|---|
| Line number | `NrWierszaFa` | integer | Yes | Sequential, starting from 1 |
| Name | `P_7` | string(512) | Yes | Product/service description (max 512 chars in FA(3)) |
| Unit | `P_8A` | string | No | Unit of measure (e.g., "szt.", "godz.", "kg") |
| Quantity | `P_8B` | decimal | No | Quantity |
| Unit price (net) | `P_9A` | decimal | No | Unit net price |
| Unit price (gross) | `P_9B` | decimal | No | Unit gross price (alternative to P_9A) |
| Discount | `P_10` | decimal | No | Discount amount |
| Line net total | `P_11` | decimal | Yes* | Net amount for this line |
| Line net (dup) | `P_11A` | decimal | No | Net amount (used for certain invoice types) |
| VAT rate | `P_12` | string | Yes | VAT rate code (see 8.6) |
| Line VAT amount | `P_12_XII` | decimal | No | VAT amount for this line |

**Maximum 10,000 line items** per invoice.

### 8.8 Payment Form Codes (FormaPlatnosci)

| Code | Meaning |
|---|---|
| 1 | Cash (gotowka) |
| 2 | Card (karta) |
| 3 | Voucher (bon) |
| 4 | Check (czek) |
| 5 | Credit (kredyt) |
| 6 | Bank transfer (przelew) |
| 7 | Mobile payment |

### 8.9 Address Format

FA(3) uses a simplified two-line address format:

```xml
<Adres>
    <KodKraju>PL</KodKraju>          <!-- ISO 3166-1 alpha-2 -->
    <AdresL1>ul. Testowa 1</AdresL1>  <!-- Street, building, apartment -->
    <AdresL2>00-001 Warszawa</AdresL2> <!-- Postal code + City -->
</Adres>
```

For foreign counterparties, use the appropriate country code and address format.

### 8.10 Correction Invoice (Faktura Korygujaca)

To issue a correction invoice, add these fields to the `Fa` section:

```xml
<Fa>
    <RodzajFaktury>KOR</RodzajFaktury>  <!-- KOR = correction -->
    <!-- ... normal invoice fields ... -->
    <FaKorygujaca>
        <NrFaKorygowanej>FV/2026/04/001</NrFaKorygowanej>
        <NrKSeFFaKorygowanej>1234567890-20260413-XXXXXXXXXX-XX</NrKSeFFaKorygowanej>
        <PrzyczynaKorekty>Blad w cenie jednostkowej</PrzyczynaKorekty>
    </FaKorygujaca>
</Fa>
```

| Field | Description |
|---|---|
| `RodzajFaktury` | `"VAT"` (standard), `"KOR"` (correction), `"ZAL"` (advance), `"KOR_ZAL"` (advance correction), `"ROZ"` (settlement) |
| `NrFaKorygowanej` | Original invoice number being corrected |
| `NrKSeFFaKorygowanej` | KSeF number of the original invoice |
| `PrzyczynaKorekty` | Reason for correction (free text) |

---

## 9. Test Data Endpoints

The KSeF test environment provides special endpoints under `/api/v2/testdata/`
for creating test entities without requiring real tax office registration.
These endpoints are **only available on the test environment** (`api-test.ksef.mf.gov.pl`).

**No authentication is required for test data endpoints.**

### 9.1 POST /api/v2/testdata/person

Creates a test person (osoba fizyczna) in the test environment.

**Request Body:**

```json
{
  "pesel": "82121200011",
  "firstName": "Jan",
  "lastName": "Testowy",
  "dateOfBirth": "1982-12-12"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `pesel` | string | Yes | 11-digit PESEL number (must pass checksum validation) |
| `firstName` | string | Yes | First name |
| `lastName` | string | Yes | Last name |
| `dateOfBirth` | string (date) | Yes | Date of birth (YYYY-MM-DD) |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:00:00.000Z",
  "referenceNumber": "20260413-TP-XXXXXXXXXX",
  "personIdentifier": {
    "pesel": "82121200011"
  }
}
```

### 9.2 POST /api/v2/testdata/subject

Creates a test subject (business entity) -- JDG, spolka, or VAT group.

**Request Body:**

```json
{
  "nip": "5213609827",
  "fullName": "Firma Testowa Sp. z o.o.",
  "subjectType": "COMPANY",
  "registrationDate": "2020-01-01"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `nip` | string | Yes | 10-digit NIP (must pass checksum) |
| `fullName` | string | Yes | Full legal name of the entity |
| `subjectType` | string | Yes | `"JDG"` (sole proprietor), `"COMPANY"` (spolka), `"VAT_GROUP"` (grupa VAT) |
| `registrationDate` | string (date) | No | Registration date |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:00:00.000Z",
  "referenceNumber": "20260413-TS-XXXXXXXXXX",
  "subjectIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  }
}
```

### 9.3 POST /api/v2/testdata/permissions

Assigns permissions (uprawnienia) to a person or certificate for a given subject.

**Request Body:**

```json
{
  "contextIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  },
  "credentialsIdentifier": {
    "type": "pesel",
    "identifier": "82121200011"
  },
  "permissionList": [
    "OWNER",
    "CREDENTIALS_MANAGE",
    "INVOICE_READ",
    "INVOICE_WRITE",
    "SELF_INVOICING"
  ]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `contextIdentifier` | object | Yes | The subject (NIP) receiving permissions |
| `credentialsIdentifier` | object | Yes | The person/cert getting the permissions |
| `permissionList` | string[] | Yes | List of permission codes |

**Permission Codes:**

| Code | Description |
|---|---|
| `OWNER` | Full access -- owns the subject in KSeF |
| `CREDENTIALS_MANAGE` | Can manage other users' permissions |
| `SUBUNIT_MANAGE` | Can manage subunits |
| `INVOICE_READ` | Can read/download invoices |
| `INVOICE_WRITE` | Can submit invoices |
| `SELF_INVOICING` | Can self-invoice (samofakturowanie) |

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:00:00.000Z",
  "referenceNumber": "20260413-TU-XXXXXXXXXX",
  "processingDescription": "Uprawnienia nadane"
}
```

### 9.4 POST /api/v2/testdata/attachment

Enables attachment support for a test subject. By default, test subjects
cannot use the attachment feature -- this endpoint activates it.

**Request Body:**

```json
{
  "contextIdentifier": {
    "type": "onip",
    "identifier": "5213609827"
  },
  "attachmentEnabled": true
}
```

**Response (200 OK):**

```json
{
  "timestamp": "2026-04-13T10:00:00.000Z",
  "referenceNumber": "20260413-TA-XXXXXXXXXX",
  "processingDescription": "Obsluga zalacznikow wlaczona"
}
```

### 9.5 Test Environment Setup Sequence

For a complete test setup, call the endpoints in this order:

```
1. POST /testdata/person      --> create the person who will sign
2. POST /testdata/subject     --> create the company (NIP)
3. POST /testdata/permissions --> grant OWNER + INVOICE_WRITE to the person
4. POST /testdata/attachment  --> (optional) enable attachments

Then proceed with normal auth flow:
5. POST /auth/challenge       --> get challenge for the NIP
6. Sign AuthTokenRequest with self-signed cert
7. POST /auth/xades-signature --> submit signed request
8. GET /auth/{ref}            --> poll for AUTHORIZED
9. POST /auth/token/redeem    --> get JWT tokens
10. POST /sessions/online     --> open session
11. POST /sessions/online/{ref}/invoices --> submit encrypted invoice
12. POST /sessions/online/{ref}/close    --> close and get UPO
```

**Important**: Test data is shared among all users of the test environment.
Use unique, unlikely NIP values to avoid collisions. The test environment is
reset periodically, so your test data may disappear.

---

## 10. Error Handling

### 10.1 HTTP Status Codes

| Status | Meaning | Typical Cause |
|---|---|---|
| 200 | Success | Normal response |
| 400 | Bad Request | Invalid request body, missing fields, validation failure |
| 401 | Unauthorized | Missing/expired/invalid access token |
| 403 | Forbidden | Insufficient permissions for the operation |
| 404 | Not Found | Invalid reference number, invoice not found |
| 409 | Conflict | Duplicate submission, session already closed |
| 413 | Payload Too Large | Invoice exceeds size limit |
| 422 | Unprocessable Entity | XML validation failure, schema mismatch |
| 429 | Too Many Requests | Rate limit exceeded |
| 500 | Internal Server Error | KSeF system error |
| 503 | Service Unavailable | Maintenance window or system overload |

### 10.2 Error Response Format

All errors follow this structure:

```json
{
  "exception": {
    "serviceCtx": "srvTESTKSEF",
    "serviceCode": "20260413-EX-XXXXXXXXXX-YYYYYYYY-ZZ",
    "serviceName": "online.session.invoice.send",
    "timestamp": "2026-04-13T10:35:00.000Z",
    "referenceNumber": null,
    "exceptionDetailList": [
      {
        "exceptionCode": 31001,
        "exceptionDescription": "Faktura nie przeszla walidacji schematu FA(3)"
      }
    ]
  }
}
```

### 10.3 Common Exception Codes

| Code | Description | Resolution |
|---|---|---|
| 10001 | Brak wymaganego pola (Missing required field) | Check request body completeness |
| 20001 | Challenge wygasl (Challenge expired) | Request a new challenge |
| 21001 | Podmiot nie istnieje (Subject not found) | Create subject via /testdata/subject or verify NIP |
| 21002 | Brak uprawnien (Insufficient permissions) | Grant permissions via /testdata/permissions or ZAW-FA |
| 21003 | Nieprawidlowy podpis (Invalid signature) | Verify XAdES signature format and certificate |
| 22001 | Token wygasl (Token expired) | Use refresh token flow |
| 22002 | Nieprawidlowy refresh token | Re-authenticate from scratch |
| 30001 | Sesja nie istnieje (Session not found) | Check reference number |
| 30002 | Sesja zamknieta (Session already closed) | Open a new session |
| 30003 | Sesja wygasla (Session expired) | Sessions expire after 12h; open new one |
| 31001 | Walidacja schematu nieudana (Schema validation failed) | Validate XML against FA(3) XSD before sending |
| 31002 | Blad deszyfrowania (Decryption error) | Check encryption: AES key, IV, RSA-OAEP params |
| 31003 | Niezgodnosc hash (Hash mismatch) | SHA-256 hash must match plaintext XML bytes |
| 31004 | Rozmiar pliku niezgodny (File size mismatch) | fileSize must equal plaintext XML byte length |
| 40001 | Limit faktur w sesji (Session invoice limit exceeded) | Max 10,000 invoices per session |

---

## 11. Rate Limits and Constraints

### 11.1 Session Constraints

| Constraint | Value |
|---|---|
| Session TTL | 12 hours |
| Max invoices per session | 10,000 |
| Max concurrent sessions per NIP | 1 (interactive), multiple (batch) |
| Max invoice size (no attachments) | 1 MB |
| Max invoice size (with attachments) | 3 MB |
| Max line items per invoice (FaWiersz) | 10,000 |
| Max Podmiot3 entries | 100 |
| Max product/service name length (P_7) | 512 characters |
| Bank account number max length | 34 characters (IBAN) |

### 11.2 Token TTL

| Token | TTL | Refresh Strategy |
|---|---|---|
| Challenge | 10 minutes | Request new one |
| Access Token (JWT) | ~15 minutes (900s) | Use refresh token |
| Refresh Token | ~7 days (604800s) | Re-authenticate fully |

### 11.3 Polling Recommendations

| Endpoint | Recommended Interval | Max Wait |
|---|---|---|
| GET /auth/{ref} | 1-2 seconds | 60 seconds (then timeout) |
| GET /sessions/{ref}/upo | 2-5 seconds | 120 seconds |

### 11.4 Test Environment Specifics

| Property | Value |
|---|---|
| Daily maintenance window | 16:00 - 18:00 CET |
| Self-signed certificates | Accepted |
| Data persistence | Periodic resets (data is not permanent) |
| Access | Open, no registration required |

---

## Appendix A: Complete Auth Flow Sequence Diagram

```
Client                                KSeF API
  |                                      |
  |  POST /auth/challenge                |
  |  { contextIdentifier: { onip, NIP } }|
  |------------------------------------->|
  |                                      |
  |  200 { challenge, timestamp }        |
  |<-------------------------------------|
  |                                      |
  |  [Build AuthTokenRequest XML]        |
  |  [Sign with XAdES-BES]              |
  |  [Base64 encode signed XML]         |
  |                                      |
  |  POST /auth/xades-signature          |
  |  { authData: "<base64>" }            |
  |------------------------------------->|
  |                                      |
  |  200 { referenceNumber }             |
  |<-------------------------------------|
  |                                      |
  |  GET /auth/{referenceNumber}         |
  |------------------------------------->|  (poll loop)
  |  200 { status: PENDING }             |
  |<-------------------------------------|
  |         ... wait 1-2s ...            |
  |  GET /auth/{referenceNumber}         |
  |------------------------------------->|
  |  200 { status: AUTHORIZED }          |
  |<-------------------------------------|
  |                                      |
  |  POST /auth/token/redeem             |
  |  { referenceNumber }                 |
  |------------------------------------->|
  |                                      |
  |  200 { accessToken, refreshToken }   |
  |<-------------------------------------|
  |                                      |
  |  [Now use Authorization: Bearer ...] |
  |                                      |
  |  POST /sessions/online               |
  |  { contextIdentifier, formCode }     |
  |------------------------------------->|
  |  200 { referenceNumber (session) }   |
  |<-------------------------------------|
  |                                      |
  |  [Encrypt invoice XML]              |
  |  [AES-256-CBC + RSA-OAEP]           |
  |                                      |
  |  POST /sessions/online/{ref}/invoices|
  |  { invoicePayload: { encrypted... } }|
  |------------------------------------->|
  |  200 { elementReferenceNumber }      |
  |<-------------------------------------|
  |                                      |
  |  POST /sessions/online/{ref}/close   |
  |------------------------------------->|
  |  200 { upoReferenceNumber }          |
  |<-------------------------------------|
  |                                      |
  |  GET /sessions/{ref}/upo             |
  |------------------------------------->|
  |  200 { upo: "<base64-UPO-XML>" }    |
  |<-------------------------------------|
```

---

## Appendix B: Rust Crate Recommendations

| Purpose | Crate | Notes |
|---|---|---|
| HTTP client | `reqwest` | Async, TLS support, JSON (de)serialization |
| JSON (de)serialization | `serde`, `serde_json` | Standard |
| XML generation | `quick-xml` | Fast, supports serialization and writing |
| XML parsing | `quick-xml` | Or `roxmltree` for read-only parsing |
| XML canonicalization | Manual or `xmlsec` bindings | exc-c14n needed for XAdES |
| AES-256-CBC encryption | `aes`, `cbc` (RustCrypto) | With `cipher` traits |
| RSA-OAEP key encryption | `rsa` (RustCrypto) | SHA-256 + MGF1-SHA256 |
| SHA-256 hashing | `sha2` (RustCrypto) | For invoice hash and XAdES digests |
| Base64 encoding | `base64` | Standard encoding |
| X.509 certificate handling | `openssl` or `rcgen` (for self-signed) | `openssl` for production certs, `rcgen` for test |
| XAdES signing | `openssl` (manual XML-DSig construction) | No pure-Rust XAdES crate exists |
| JWT parsing (optional) | `jsonwebtoken` | To inspect access token claims/expiry |
| Random bytes | `rand` with `OsRng` | For AES key and IV generation |
| Date/time | `chrono` | ISO 8601 timestamps |
| Error types | `thiserror` | Derive Error trait |
| Async runtime | `tokio` | With `reqwest` |

---

## Appendix C: NIP Checksum Validation

Polish NIP (Numer Identyfikacji Podatkowej) is a 10-digit number with a checksum.

```rust
fn validate_nip(nip: &str) -> bool {
    if nip.len() != 10 || !nip.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    let digits: Vec<u32> = nip.chars().map(|c| c.to_digit(10).unwrap()).collect();
    let weights = [6, 5, 7, 2, 3, 4, 5, 6, 7];

    let checksum: u32 = digits[..9]
        .iter()
        .zip(weights.iter())
        .map(|(d, w)| d * w)
        .sum();

    checksum % 11 == digits[9]
}
```

---

## Appendix D: Differences from KSeF 1.0

If referencing KSeF 1.0 code or documentation, be aware of these breaking changes:

| Aspect | KSeF 1.0 | KSeF 2.0 |
|---|---|---|
| Invoice schema | FA(1), FA(2) | FA(3) only (on prod/demo) |
| Auth tokens | Simple session token | JWT (accessToken + refreshToken) |
| Token exchange | Immediate after auth | Separate `/auth/token/redeem` step |
| KSeF tokens | Primary auth method | Deprecated (removed Jan 2027) |
| Encryption | Optional in early versions | Mandatory always |
| API version prefix | `/api/v1/` | `/api/v2/` |
| Attachments | Not supported | Supported via `Zalacznik` element |
| Product name length | 256 chars | 512 chars |

---

## Appendix E: Quick Reference -- Header Values for formCode

When opening a session, the `formCode` object identifies which schema you're using:

```json
{
  "systemCode": "FA (3)",
  "schemaVersion": "1-0E",
  "targetNamespace": "http://crd.gov.pl/wzor/2025/06/25/13775/",
  "value": "FA"
}
```

These values must match exactly. Using wrong values will result in schema
validation errors when submitting invoices.
