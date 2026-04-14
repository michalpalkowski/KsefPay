# Invoice Data Flow — Research & MVP Decision

## Context

We need to determine where invoice data comes from in our Rust (Axum + Askama) KSeF
integration app. This document covers how small businesses create invoices, what data
is mandatory for FA(3), what lookup APIs exist, and what our MVP approach should be.

---

## 1. How Small Businesses Create Invoices for KSeF

There are three distinct user profiles, each with a different workflow:

### Profile A: Uses Accounting Software (Symfonia, Optima, enova, Sage, Insert)

- The ERP/accounting system generates invoices internally.
- Modern versions (2026+) have built-in KSeF integration — they generate FA(3) XML,
  encrypt it, and submit it to KSeF directly from the application.
- These users **do not need our app** for invoice creation. They might use it for
  monitoring, receiving purchase invoices, or as a fallback.
- Export format: typically proprietary internal format. Some support FA(3) XML export,
  but this is not standardized across vendors.

### Profile B: Uses Online Invoicing Services (iFirma, Fakturownia, InFakt, wFirma)

- SaaS platforms handle everything: invoice creation via web forms, FA(3) XML
  generation, KSeF submission.
- These users also **do not need our app** unless they want an independent tool.
- The web form UX in these services is a good reference for our own form design.

### Profile C: Single-Person Business / Micro-Entrepreneur (Our Target User)

- Has no ERP system or only basic spreadsheet-based bookkeeping.
- Before KSeF, would create invoices in Word/Excel or a free invoice generator.
- Now needs KSeF compliance but does not want to pay for accounting software.
- **This user types invoice data into a web form.** They know their own company data,
  the buyer's NIP, what they sold, and the amounts. They need a simple form that
  generates the XML for them.
- Some of these users might also receive FA(3) XML files from their accountant
  (biuro rachunkowe) and want to upload them directly.

### Conclusion

Our primary user creates invoices by **filling in a web form**. XML upload is a
secondary/power-user feature. We should support both, but the form is the core flow.

---

## 2. Minimum Data for a Valid FA(3) Invoice

Based on the FA(3) XSD schema (namespace `http://crd.gov.pl/wzor/2025/06/25/13775/`)
and the official documentation from podatki.gov.pl:

### Mandatory Elements

#### Naglowek (Header)
| Field | XML Element | Description | Example |
|-------|-------------|-------------|---------|
| Schema version | `KodFormularza/@wersjaSchemy` | Always "1-0" for FA(3) | `1-0` |
| Form code | `KodFormularza` | Always "FA" with kodSystemowy="FA (3)" | `FA` |
| Variant | `WariantFormularza` | Always "3" | `3` |
| Creation date | `DataWytworzeniaFa` | ISO datetime when XML was generated | `2026-04-13T10:00:00` |
| System name | `SystemInfo` | Name of the issuing system (max 256 chars) | `ksef-paymoney` |

#### Podmiot1 (Seller) — Mandatory
| Field | XML Element | Description | Example |
|-------|-------------|-------------|---------|
| Seller NIP | `DaneIdentyfikacyjne/NIP` | 10-digit Polish tax ID | `1234567890` |
| Full name | `DaneIdentyfikacyjne/Nazwa` | Company name (max 512 chars) | `Firma ABC Sp. z o.o.` |
| Country code | `Adres/KodKraju` | ISO 3166-1 alpha-2 | `PL` |
| Street + number | `Adres/AdresL1` | Street address line 1 (max 256 chars) | `ul. Testowa 1` |
| City + postal | `Adres/AdresL2` | City and postal code (max 256 chars) | `00-001 Warszawa` |

Note: FA(3) uses a simplified address format (AdresL1 + AdresL2) instead of the
structured address from FA(2) (Ulica/NrDomu/NrLokalu/Miejscowosc/KodPocztowy).

#### Podmiot2 (Buyer) — Mandatory
Same structure as Podmiot1. For domestic B2B transactions, NIP is required.
For B2C (consumers), NIP can be omitted and replaced with other identification.

#### Fa (Invoice Core) — Mandatory
| Field | XML Element | Description | Example |
|-------|-------------|-------------|---------|
| Invoice type | `KodWaluty` | Currency code (ISO 4217) | `PLN` |
| Invoice date | `P_1` | Issue date (YYYY-MM-DD) | `2026-04-13` |
| Invoice number | `P_2` | Sequential number assigned by seller | `FV/2026/04/001` |
| Sale date | `P_6` | Date of supply/service (can equal P_1) | `2026-04-13` |
| Payment deadline | `TerminPlatnosci/Termin` | Payment due date | `2026-04-27` |
| Payment method | `FormaPlatnosci` | Payment method code (1-6) | `6` (przelew/transfer) |

**VAT Summary (at least one rate block required):**
| Field | XML Element | Description |
|-------|-------------|-------------|
| Net amount at 23% | `P_13_1` | Net total for items at 23% VAT |
| VAT amount at 23% | `P_14_1` | VAT total for items at 23% VAT |
| Net amount at 8% | `P_13_2` | Net total for items at 8% VAT |
| VAT amount at 8% | `P_14_2` | VAT total for items at 8% VAT |
| Net amount at 5% | `P_13_3` | Net total for items at 5% VAT |
| VAT amount at 5% | `P_14_3` | VAT total for items at 5% VAT |
| Net amount at 0% | `P_13_6_1` | Net total for items at 0% VAT |
| Exempt net amount | `P_13_7` | Net total for VAT-exempt items |

Only the rate blocks that appear on the invoice are required. A typical service
invoice with only 23% VAT needs only P_13_1 and P_14_1.

**Totals:**
| Field | XML Element | Description |
|-------|-------------|-------------|
| Total net | `P_15` | Sum of all net amounts |
| Total gross | `P_15ZL` | Total gross amount (net + VAT), always in PLN |

**Line Items (FaWiersz) — at least one required:**
| Field | XML Element | Description | Example |
|-------|-------------|-------------|---------|
| Line number | `NrWierszaFa` | Sequential (1, 2, 3...) | `1` |
| Item name | `P_7` | Description (max 512 chars in FA(3)) | `Uslugi programistyczne` |
| Unit of measure | `P_8A` | Optional but recommended | `usl` (uslugi) |
| Quantity | `P_8B` | Quantity | `160` |
| Unit net price | `P_9A` | Net price per unit | `150.00` |
| Net value | `P_11` | Line net total (P_8B * P_9A) | `24000.00` |
| VAT rate | `P_12` | VAT rate as integer (23, 8, 5, 0) or "zw" | `23` |
| VAT amount | `P_11Vat` | Line VAT amount | `5520.00` |
| Gross value | `P_11Brutto` | Line gross total | `29520.00` |

#### Bank Account (conditional — required when payment method is transfer)
| Field | XML Element | Description |
|-------|-------------|-------------|
| Account number | `RachunekBankowy/NrRB` | IBAN or domestic account number (max 34 chars) |

### What Is NOT Required (Can Be Omitted in MVP)

- Podmiot3 (additional parties) — only for special cases
- PodmiotUpowazniony — only when a third party issues the invoice
- Stopka — free-text footer, optional
- Zalacznik — attachments, new in FA(3), optional
- Correction fields (P_15ZK etc.) — only for correction invoices
- Split payment annotation (MPP) — only when mandatory split payment applies
- GTU codes — goods/services type codes, optional since 2024
- Related document references — optional

### Minimal Valid Invoice Summary

A minimal B2B service invoice needs exactly:
1. Naglowek: version info + creation timestamp + system name
2. Podmiot1: seller NIP + name + address (2 lines)
3. Podmiot2: buyer NIP + name + address (2 lines)
4. Fa: invoice number, dates (issue + sale), currency, payment terms,
   one VAT rate summary block, totals, at least one line item
5. Bank account number (if payment method is transfer)

**That is approximately 25-30 form fields** for a simple one-line-item invoice.

---

## 3. Can Invoice Data Be Fetched by NIP?

Yes. There are several APIs that allow looking up company data by NIP:

### GUS REGON (BIR1) — Official Statistics Office

- **URL**: `https://wyszukiwarkaregon.stat.gov.pl/appBIR/UslugaBIRz662.svc`
- **Protocol**: SOAP/XML (unfortunately not REST)
- **Data returned**: Full company name, REGON number, address (street, building number,
  apartment number, city, postal code, voivodeship), legal form, PKD codes
- **Access**: Free, requires a one-time API key request via the GUS website
  (`https://api.stat.gov.pl/Home/RegonApi`). Test key: `abcde12345abcde12345`
- **Rate limits**: Reasonable for our use case (no published hard limit, but
  session-based — one session at a time)
- **Reliability**: Generally good but can be slow. The SOAP interface is clunky.

### VIES (VAT Information Exchange System) — EU-Wide

- **URL**: `https://ec.europa.eu/taxation_customs/vies/rest-api/check-vat-number`
- **Protocol**: REST/JSON (new API since 2024, replaces old SOAP)
- **Data returned**: Company name, address (single string), VAT validity status
- **Access**: Free, no API key needed
- **Limitations**: Only confirms EU VAT number validity + returns name/address.
  Address is a single unstructured string — not ideal for form pre-fill.
  Only works for EU-registered VAT payers.

### Ministerstwo Finansow — Biala Lista (White List of VAT Payers)

- **URL**: `https://wl-api.mf.gov.pl/api/search/nip/{nip}?date={YYYY-MM-DD}`
- **Protocol**: REST/JSON
- **Data returned**: Company name, NIP, REGON, KRS, registered address, bank account
  numbers (crucial for invoice verification!), VAT registration status
- **Access**: Free, no API key needed
- **Rate limits**: 10 requests per second
- **This is the best option for our use case** because:
  - REST/JSON (easy to integrate)
  - Returns bank account numbers (needed on invoices!)
  - Returns full structured address
  - Confirms active VAT payer status
  - No registration needed

### Recommended Approach for MVP

1. **Seller data**: Store once during initial setup (NIP, name, address, bank account).
   Pre-fill from Biala Lista API on first setup.
2. **Buyer data**: When user enters a NIP in the invoice form, call Biala Lista API
   to auto-fill buyer name + address. Cache results locally.
3. **Validation**: Use Biala Lista to verify the buyer is an active VAT payer before
   submitting the invoice.

### Example Biala Lista API Call

```
GET https://wl-api.mf.gov.pl/api/search/nip/1234567890?date=2026-04-13

Response:
{
  "result": {
    "subject": {
      "name": "FIRMA ABC SPOLKA Z OGRANICZONA ODPOWIEDZIALNOSCIA",
      "nip": "1234567890",
      "regon": "123456789",
      "residenceAddress": "ul. Testowa 1, 00-001 Warszawa",
      "workingAddress": "ul. Testowa 1, 00-001 Warszawa",
      "accountNumbers": [
        "PL12345678901234567890123456"
      ],
      "statusVat": "Czynny"
    }
  }
}
```

Note: The address is returned as a single string. We will need to parse it or let
the user adjust the split into AdresL1/AdresL2 for FA(3).

---

## 4. What Format Do ERP Systems Export?

### The Situation Is Fragmented

- **No universal export format** exists across Polish ERP systems.
- Each ERP (Symfonia, Optima, enova, Sage, Insert, Subiekt) uses its own internal
  data model and database schema.
- Modern ERP versions (2025+) have built-in KSeF modules that generate FA(3) XML
  internally and submit directly — they do not expose FA(3) XML to the user.

### FA(3) XML as the De Facto Standard

- Since KSeF became mandatory, FA(3) XML is effectively the interchange format.
- Some ERPs allow exporting invoices as FA(3) XML files (for backup, migration,
  or submission through alternative channels).
- The official KSeF Aplikacja Podatnika (`ap.ksef.mf.gov.pl`) accepts FA(3) XML
  uploads directly — this establishes FA(3) as the standard import format.
- Accountants (biura rachunkowe) sometimes exchange FA(3) XML files with clients.

### Other Formats (Less Relevant for MVP)

- **JPK_FA** (Jednolity Plik Kontrolny - Faktury): XML format for tax audit purposes.
  Different schema from FA(3). Not used for KSeF submission.
- **CSV/Excel**: Some simple tools export line items as CSV. Would need transformation
  to FA(3). Not worth supporting in MVP.
- **Peppol BIS Billing 3.0 (UBL)**: EU cross-border e-invoicing format. Poland is
  integrating KSeF with Peppol, but this is a future concern.

### Conclusion for Our App

If we support import, **FA(3) XML is the only format worth supporting**. It is the
canonical format, the one KSeF itself uses, and the one an accountant would hand you.

---

## 5. MVP Decision: The Invoice Creation Flow

### Recommendation: Option C (Both), with Form as Primary

The MVP should support two ways to create an invoice:

#### Primary Flow: Web Form (for Profile C users)

```
User opens /invoices/new
  |
  v
Form with sections:
  1. Seller data (pre-filled from saved profile, editable)
  2. Buyer NIP field --> [Lookup] button --> auto-fills name + address via Biala Lista
  3. Invoice metadata (number auto-generated, dates default to today, payment terms)
  4. Line items (dynamic add/remove rows)
  5. VAT summary (auto-calculated from line items)
  6. Bank account (pre-filled from profile)
  |
  v
User clicks "Submit" --> app generates FA(3) XML --> encrypts --> sends to KSeF
```

**Form field count for a one-item invoice**: ~15 fields the user actually fills in
(buyer NIP triggers auto-fill, seller is pre-saved, dates are defaulted, amounts
are auto-calculated). The rest is computed.

What the user actually types:
- Buyer NIP (rest auto-filled)
- Invoice number (or accept auto-generated)
- Item description
- Quantity
- Unit price
- VAT rate (dropdown, default 23%)
- Payment deadline (date picker, default +14 days)

That is **7 fields** for the simplest case. Very achievable for MVP.

#### Secondary Flow: XML Upload (for power users / accountants)

```
User opens /invoices/upload
  |
  v
File upload form (drag & drop or file picker)
  |
  v
App validates FA(3) XML against schema
  |
  v
Shows preview of parsed invoice data
  |
  v
User confirms --> app encrypts --> sends to KSeF
```

This is simpler to implement (no form logic, just parse + validate + submit) and
serves users who get XML from their accountant or another system.

### Implementation Priority

| Priority | Feature | Effort | Value |
|----------|---------|--------|-------|
| P0 | Seller profile setup (save once) | Small | Enables form pre-fill |
| P0 | Invoice web form (manual entry) | Medium | Core MVP feature |
| P0 | FA(3) XML generation from form data | Medium | Required for submission |
| P1 | Buyer NIP lookup via Biala Lista API | Small | Major UX improvement |
| P1 | Auto-calculation of VAT/totals | Small | Prevents errors |
| P2 | FA(3) XML upload + validation | Small | Power user feature |
| P2 | Buyer data caching (remember past buyers) | Small | Repeat invoicing UX |
| P3 | Invoice templates / cloning | Small | Convenience |
| P3 | Invoice numbering auto-increment | Small | Convenience |

### Architecture Implications

The form approach means we need:

1. **Domain types for invoice creation** (not just the KSeF submission types):
   - `InvoiceFormData` struct matching the form fields
   - `SellerProfile` persisted in DB
   - `BuyerCache` for remembered buyers

2. **FA(3) XML builder**: `InvoiceFormData -> FA(3) XML string`
   - This is distinct from FA(3) XML parsing (which is for incoming invoices)
   - Must produce schema-valid XML

3. **Biala Lista HTTP client**: simple GET request, deserialize JSON response
   - New port trait: `CompanyLookup` with method `lookup_by_nip(nip) -> CompanyData`
   - Implementation: `BialaListaClient` using reqwest

4. **Form validation layer**: validate all fields before generating XML
   - NIP checksum validation
   - Amount consistency (line items sum to totals)
   - Required field presence

### Data Model Addition

```sql
-- Seller profile (the user's own company data, saved once)
CREATE TABLE seller_profiles (
    id UUID PRIMARY KEY,
    nip VARCHAR(10) NOT NULL UNIQUE,
    name VARCHAR(512) NOT NULL,
    address_line1 VARCHAR(256) NOT NULL,
    address_line2 VARCHAR(256) NOT NULL,
    bank_account VARCHAR(34),
    is_default BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Cached buyer data (from Biala Lista lookups)
CREATE TABLE buyer_cache (
    nip VARCHAR(10) PRIMARY KEY,
    name VARCHAR(512) NOT NULL,
    address_line1 VARCHAR(256),
    address_line2 VARCHAR(256),
    status_vat VARCHAR(32),
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### New Port Trait

```rust
/// Port: Company data lookup by NIP
#[async_trait]
pub trait CompanyLookup: Send + Sync {
    async fn lookup_by_nip(&self, nip: &Nip) -> Result<CompanyData, LookupError>;
}

pub struct CompanyData {
    pub nip: Nip,
    pub name: String,
    pub address: String,          // raw from API
    pub bank_accounts: Vec<String>,
    pub vat_status: VatStatus,    // Active, Exempt, NotRegistered
}
```

---

## Summary of Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Primary invoice creation method | Web form | Our target user has no ERP; needs a simple UI |
| Secondary method | FA(3) XML upload | Low effort, serves accountant handoff scenario |
| Buyer data pre-fill | Biala Lista API | Free, REST/JSON, returns name + address + bank accounts |
| Seller data | Stored profile, entered once | Small business has one seller identity |
| Export format to support | FA(3) XML only | It is the KSeF standard; no other format is worth the effort |
| Form complexity for MVP | ~7 user-entered fields for simplest invoice | Buyer auto-filled, seller pre-saved, amounts calculated |
