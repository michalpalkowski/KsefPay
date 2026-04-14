-- KSeF auth tokens (JWT-based)
CREATE TABLE IF NOT EXISTS ksef_auth_tokens (
    id UUID PRIMARY KEY,
    nip VARCHAR(10) NOT NULL,
    environment VARCHAR(10) NOT NULL CHECK (environment IN ('test', 'demo', 'production')),
    access_token TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_token_expires_at TIMESTAMPTZ NOT NULL,
    refresh_token_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_auth_tokens_nip_env ON ksef_auth_tokens(nip, environment);

-- KSeF interactive sessions
CREATE TABLE IF NOT EXISTS ksef_sessions (
    id UUID PRIMARY KEY,
    session_reference VARCHAR(256) NOT NULL UNIQUE,
    nip VARCHAR(10) NOT NULL,
    environment VARCHAR(10) NOT NULL CHECK (environment IN ('test', 'demo', 'production')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    terminated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_sessions_nip_env ON ksef_sessions(nip, environment);
CREATE INDEX IF NOT EXISTS idx_sessions_active ON ksef_sessions(nip, environment) WHERE terminated_at IS NULL;

-- Invoices (both sent and received)
CREATE TABLE IF NOT EXISTS invoices (
    id UUID PRIMARY KEY,
    direction VARCHAR(8) NOT NULL CHECK (direction IN ('outgoing', 'incoming')),
    status VARCHAR(16) NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'queued', 'submitted', 'accepted', 'rejected', 'failed')),
    invoice_number VARCHAR(64) NOT NULL,
    issue_date DATE NOT NULL,
    sale_date DATE NOT NULL,
    seller_nip VARCHAR(10) NOT NULL,
    seller_name VARCHAR(512) NOT NULL,
    seller_country VARCHAR(2) NOT NULL,
    seller_address_line1 VARCHAR(256) NOT NULL,
    seller_address_line2 VARCHAR(256) NOT NULL,
    buyer_nip VARCHAR(10) NOT NULL,
    buyer_name VARCHAR(512) NOT NULL,
    buyer_country VARCHAR(2) NOT NULL,
    buyer_address_line1 VARCHAR(256) NOT NULL,
    buyer_address_line2 VARCHAR(256) NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'PLN',
    line_items JSONB NOT NULL,
    total_net_grosze BIGINT NOT NULL,
    total_vat_grosze BIGINT NOT NULL,
    total_gross_grosze BIGINT NOT NULL,
    payment_method SMALLINT NOT NULL,
    payment_deadline DATE NOT NULL,
    bank_account VARCHAR(34),
    ksef_number VARCHAR(128) UNIQUE,
    ksef_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_seller_nip ON invoices(seller_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_buyer_nip ON invoices(buyer_nip);
CREATE INDEX IF NOT EXISTS idx_invoices_direction ON invoices(direction);

-- Job queue for async processing
CREATE TABLE IF NOT EXISTS jobs (
    id UUID PRIMARY KEY,
    job_type VARCHAR(32) NOT NULL,
    payload JSONB NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'dead_letter')),
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    last_error TEXT,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_jobs_dequeue ON jobs(status, scheduled_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_jobs_dead_letter ON jobs(status) WHERE status = 'dead_letter';
