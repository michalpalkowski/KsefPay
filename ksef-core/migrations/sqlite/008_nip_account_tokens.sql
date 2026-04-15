-- Local token registry: track which tokens were generated from which NIP account and user.
-- This gives per-NIP / per-user token isolation in the UI, independent of what the KSeF
-- API returns for the authenticated session.

CREATE TABLE IF NOT EXISTS nip_account_tokens (
    id TEXT PRIMARY KEY,
    nip_account_id TEXT NOT NULL REFERENCES nip_accounts(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    ksef_token_id TEXT NOT NULL UNIQUE,
    permissions TEXT NOT NULL DEFAULT '[]',
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    revoked_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_nip_account_tokens_account ON nip_account_tokens(nip_account_id);
