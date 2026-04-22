CREATE TABLE IF NOT EXISTS trusted_application_email_access (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    consumed_at TEXT,
    revoked_at TEXT,
    created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    CHECK (consumed_at IS NULL OR revoked_at IS NULL)
);

CREATE INDEX IF NOT EXISTS idx_trusted_application_email_access_created_by
    ON trusted_application_email_access(created_by_user_id);

CREATE INDEX IF NOT EXISTS idx_trusted_application_email_access_email
    ON trusted_application_email_access(email);

CREATE UNIQUE INDEX IF NOT EXISTS uq_trusted_application_email_access_active_email
    ON trusted_application_email_access(email)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;
