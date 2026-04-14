-- Multi-tenant auth: users, NIP accounts, access control, HTTP sessions.

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS nip_accounts (
    id UUID PRIMARY KEY,
    nip VARCHAR(10) NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    ksef_auth_method TEXT NOT NULL DEFAULT 'xades'
        CHECK (ksef_auth_method IN ('xades', 'token')),
    ksef_auth_token TEXT,
    cert_pem TEXT,
    key_pem TEXT,
    cert_auto_generated BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS user_nip_access (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nip_account_id UUID NOT NULL REFERENCES nip_accounts(id) ON DELETE CASCADE,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, nip_account_id)
);

CREATE INDEX IF NOT EXISTS idx_user_nip_access_user ON user_nip_access(user_id);
CREATE INDEX IF NOT EXISTS idx_user_nip_access_nip ON user_nip_access(nip_account_id);

-- Add nip_account_id to invoices (nullable for migration, but enforced in code)
ALTER TABLE invoices ADD COLUMN IF NOT EXISTS nip_account_id UUID REFERENCES nip_accounts(id);
CREATE INDEX IF NOT EXISTS idx_invoices_nip_account ON invoices(nip_account_id);

-- Add nip to jobs for worker context
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS nip VARCHAR(10);

-- tower-sessions storage
CREATE TABLE IF NOT EXISTS tower_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    data BYTEA NOT NULL,
    expiry_date TIMESTAMPTZ NOT NULL
);
