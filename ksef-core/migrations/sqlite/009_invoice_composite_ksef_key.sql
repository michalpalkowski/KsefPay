-- SQLite cannot drop a column-level UNIQUE constraint directly.
-- Recreate the invoices table with the uniqueness scope changed from
-- UNIQUE(ksef_number) to UNIQUE(ksef_number, nip_account_id).

PRAGMA foreign_keys = OFF;

CREATE TABLE invoices_new (
    id TEXT PRIMARY KEY,
    direction TEXT NOT NULL CHECK (direction IN ('outgoing', 'incoming')),
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'queued', 'submitted', 'accepted', 'rejected', 'failed', 'fetched')),
    invoice_type TEXT NOT NULL,
    invoice_number TEXT NOT NULL,
    issue_date TEXT NOT NULL,
    sale_date TEXT,
    corrected_invoice_number TEXT,
    correction_reason TEXT,
    original_ksef_number TEXT,
    advance_payment_date TEXT,
    seller_nip TEXT,
    seller_name TEXT NOT NULL,
    seller_country TEXT NOT NULL,
    seller_address_line1 TEXT NOT NULL,
    seller_address_line2 TEXT NOT NULL,
    buyer_nip TEXT,
    buyer_name TEXT NOT NULL,
    buyer_country TEXT NOT NULL,
    buyer_address_line1 TEXT NOT NULL,
    buyer_address_line2 TEXT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'PLN',
    line_items TEXT NOT NULL CHECK (json_valid(line_items)),
    total_net_grosze INTEGER NOT NULL,
    total_vat_grosze INTEGER NOT NULL,
    total_gross_grosze INTEGER NOT NULL,
    payment_method INTEGER,
    payment_deadline TEXT,
    bank_account TEXT,
    ksef_number TEXT,
    ksef_error TEXT,
    raw_xml TEXT,
    nip_account_id TEXT REFERENCES nip_accounts(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE (ksef_number, nip_account_id)
);

INSERT INTO invoices_new SELECT * FROM invoices;

DROP TABLE invoices;
ALTER TABLE invoices_new RENAME TO invoices;

CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_seller_nip ON invoices(seller_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_buyer_nip ON invoices(buyer_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_direction ON invoices(direction);
CREATE INDEX IF NOT EXISTS idx_invoices_invoice_type ON invoices(invoice_type);
CREATE INDEX IF NOT EXISTS idx_invoices_nip_account ON invoices(nip_account_id);

PRAGMA foreign_keys = ON;
