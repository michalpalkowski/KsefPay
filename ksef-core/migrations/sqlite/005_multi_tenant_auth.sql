-- Multi-tenant auth: users, NIP accounts, access control, HTTP sessions.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS nip_accounts (
    id TEXT PRIMARY KEY,
    nip TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    ksef_auth_method TEXT NOT NULL DEFAULT 'xades'
        CHECK (ksef_auth_method IN ('xades', 'token')),
    ksef_auth_token TEXT,
    cert_pem TEXT,
    key_pem TEXT,
    cert_auto_generated INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS user_nip_access (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nip_account_id TEXT NOT NULL REFERENCES nip_accounts(id) ON DELETE CASCADE,
    granted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (user_id, nip_account_id)
);

CREATE INDEX IF NOT EXISTS idx_user_nip_access_user ON user_nip_access(user_id);
CREATE INDEX IF NOT EXISTS idx_user_nip_access_nip ON user_nip_access(nip_account_id);

-- Add nip_account_id to invoices
ALTER TABLE invoices ADD COLUMN nip_account_id TEXT REFERENCES nip_accounts(id);
CREATE INDEX IF NOT EXISTS idx_invoices_nip_account ON invoices(nip_account_id);

-- Add nip to jobs for worker context
ALTER TABLE jobs ADD COLUMN nip TEXT;

-- tower-sessions storage
CREATE TABLE IF NOT EXISTS tower_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    data BLOB NOT NULL,
    expiry_date TEXT NOT NULL
);
