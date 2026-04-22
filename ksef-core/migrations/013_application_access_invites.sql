CREATE TABLE IF NOT EXISTS application_access_invites (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_application_access_invites_email
    ON application_access_invites(email);

CREATE INDEX IF NOT EXISTS idx_application_access_invites_created_by
    ON application_access_invites(created_by_user_id);
