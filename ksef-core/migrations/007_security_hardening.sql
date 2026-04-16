-- Security hardening: audit log + enforce invoice tenant ownership.

-- Append-only audit log table.
CREATE TABLE IF NOT EXISTS audit_log (
    id UUID PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    user_id UUID NOT NULL,
    user_email TEXT NOT NULL,
    nip VARCHAR(10),
    action TEXT NOT NULL,
    details TEXT,
    ip_address TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_nip ON audit_log(nip);
CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp);

-- Phase 0 requirement: every invoice must belong to an account.
-- Backfill should run before this migration on existing data.
ALTER TABLE invoices
    ALTER COLUMN nip_account_id SET NOT NULL;
