-- Company data cache (Biała Lista VAT) and invoice auto-numbering.

CREATE TABLE IF NOT EXISTS company_cache (
    nip             TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    address         TEXT NOT NULL,
    bank_accounts   TEXT NOT NULL DEFAULT '[]',
    vat_status      TEXT NOT NULL,
    fetched_at      TEXT NOT NULL,
    raw_response    TEXT
);

CREATE TABLE IF NOT EXISTS invoice_sequences (
    seller_nip      TEXT NOT NULL,
    year            INTEGER NOT NULL,
    month           INTEGER NOT NULL,
    last_number     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (seller_nip, year, month)
);
