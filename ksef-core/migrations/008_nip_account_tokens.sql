-- Local token registry: track which tokens were generated from which NIP account and user.
-- This gives per-NIP / per-user token isolation in the UI, independent of what the KSeF
-- API returns for the authenticated session.

CREATE TABLE IF NOT EXISTS nip_account_tokens (
    id UUID PRIMARY KEY,
    nip_account_id UUID NOT NULL REFERENCES nip_accounts(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ksef_token_id TEXT NOT NULL UNIQUE,
    permissions JSONB NOT NULL DEFAULT '[]'::jsonb,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_nip_account_tokens_account ON nip_account_tokens(nip_account_id);
CREATE INDEX IF NOT EXISTS idx_nip_account_tokens_user ON nip_account_tokens(user_id);
