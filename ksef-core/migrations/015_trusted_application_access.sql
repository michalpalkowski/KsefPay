CREATE TABLE IF NOT EXISTS trusted_application_email_access (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL,
    consumed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (consumed_at IS NULL OR revoked_at IS NULL)
);

CREATE INDEX IF NOT EXISTS idx_trusted_application_email_access_created_by
    ON trusted_application_email_access(created_by_user_id);

CREATE INDEX IF NOT EXISTS idx_trusted_application_email_access_email
    ON trusted_application_email_access(email);

CREATE UNIQUE INDEX IF NOT EXISTS uq_trusted_application_email_access_active_email
    ON trusted_application_email_access(email)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;
