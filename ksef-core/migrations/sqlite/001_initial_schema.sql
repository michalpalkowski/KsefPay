-- SQLite schema equivalent for KSeF persistence.

CREATE TABLE IF NOT EXISTS ksef_auth_tokens (
    id TEXT PRIMARY KEY,
    nip TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'demo', 'production')),
    access_token TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_token_expires_at TEXT NOT NULL,
    refresh_token_expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_auth_tokens_nip_env ON ksef_auth_tokens(nip, environment);

CREATE TABLE IF NOT EXISTS ksef_sessions (
    id TEXT PRIMARY KEY,
    session_reference TEXT NOT NULL UNIQUE,
    nip TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'demo', 'production')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    expires_at TEXT NOT NULL,
    terminated_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_nip_env ON ksef_sessions(nip, environment);
CREATE INDEX IF NOT EXISTS idx_sessions_active ON ksef_sessions(nip, environment) WHERE terminated_at IS NULL;

CREATE TABLE IF NOT EXISTS invoices (
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
    nip_account_id TEXT REFERENCES nip_accounts(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_seller_nip ON invoices(seller_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_buyer_nip ON invoices(buyer_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_direction ON invoices(direction);
CREATE INDEX IF NOT EXISTS idx_invoices_invoice_type ON invoices(invoice_type);
CREATE INDEX IF NOT EXISTS idx_invoices_nip_account ON invoices(nip_account_id);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    job_type TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'dead_letter')),
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error TEXT,
    nip TEXT,
    scheduled_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_jobs_dequeue ON jobs(status, scheduled_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_jobs_dead_letter ON jobs(status) WHERE status = 'dead_letter';
