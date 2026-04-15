-- Security hardening: audit log + enforce invoice tenant ownership.

CREATE TABLE IF NOT EXISTS audit_log (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    user_id TEXT NOT NULL,
    user_email TEXT NOT NULL,
    nip TEXT,
    action TEXT NOT NULL,
    details TEXT,
    ip_address TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_nip ON audit_log(nip);
CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp);

-- Best-effort backfill from seller/buyer NIP for legacy rows.
UPDATE invoices
SET nip_account_id = CASE
    WHEN direction = 'outgoing' THEN (
        SELECT id FROM nip_accounts WHERE nip = invoices.seller_nip LIMIT 1
    )
    WHEN direction = 'incoming' THEN (
        SELECT id FROM nip_accounts WHERE nip = invoices.buyer_nip LIMIT 1
    )
    ELSE (
        SELECT id FROM nip_accounts WHERE nip = invoices.seller_nip LIMIT 1
    )
END
WHERE nip_account_id IS NULL;

-- SQLite requires table rebuild to enforce NOT NULL.
CREATE TABLE IF NOT EXISTS invoices_new (
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
    ksef_number TEXT UNIQUE,
    ksef_error TEXT,
    raw_xml TEXT,
    nip_account_id TEXT NOT NULL REFERENCES nip_accounts(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT INTO invoices_new (
    id,
    direction,
    status,
    invoice_type,
    invoice_number,
    issue_date,
    sale_date,
    corrected_invoice_number,
    correction_reason,
    original_ksef_number,
    advance_payment_date,
    seller_nip,
    seller_name,
    seller_country,
    seller_address_line1,
    seller_address_line2,
    buyer_nip,
    buyer_name,
    buyer_country,
    buyer_address_line1,
    buyer_address_line2,
    currency,
    line_items,
    total_net_grosze,
    total_vat_grosze,
    total_gross_grosze,
    payment_method,
    payment_deadline,
    bank_account,
    ksef_number,
    ksef_error,
    raw_xml,
    nip_account_id,
    created_at,
    updated_at
)
SELECT
    id,
    direction,
    status,
    invoice_type,
    invoice_number,
    issue_date,
    sale_date,
    corrected_invoice_number,
    correction_reason,
    original_ksef_number,
    advance_payment_date,
    seller_nip,
    seller_name,
    seller_country,
    seller_address_line1,
    seller_address_line2,
    buyer_nip,
    buyer_name,
    buyer_country,
    buyer_address_line1,
    buyer_address_line2,
    currency,
    line_items,
    total_net_grosze,
    total_vat_grosze,
    total_gross_grosze,
    payment_method,
    payment_deadline,
    bank_account,
    ksef_number,
    ksef_error,
    raw_xml,
    nip_account_id,
    created_at,
    updated_at
FROM invoices;

DROP TABLE invoices;
ALTER TABLE invoices_new RENAME TO invoices;

CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_seller_nip ON invoices(seller_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_buyer_nip ON invoices(buyer_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_direction ON invoices(direction);
CREATE INDEX IF NOT EXISTS idx_invoices_invoice_type ON invoices(invoice_type);
CREATE INDEX IF NOT EXISTS idx_invoices_nip_account ON invoices(nip_account_id);
