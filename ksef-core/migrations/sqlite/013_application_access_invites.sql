CREATE TABLE IF NOT EXISTS application_access_invites (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    accepted_at TEXT,
    revoked_at TEXT,
    created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_application_access_invites_email
    ON application_access_invites(email);

CREATE INDEX IF NOT EXISTS idx_application_access_invites_created_by
    ON application_access_invites(created_by_user_id);
