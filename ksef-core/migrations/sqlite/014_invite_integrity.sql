PRAGMA foreign_keys=OFF;

CREATE TABLE workspace_invites_new (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'operator', 'read_only')),
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    accepted_at TEXT,
    revoked_at TEXT,
    created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CONSTRAINT workspace_invites_terminal_state_check
        CHECK (accepted_at IS NULL OR revoked_at IS NULL)
);

INSERT INTO workspace_invites_new (
    id,
    workspace_id,
    email,
    role,
    token_hash,
    expires_at,
    accepted_at,
    revoked_at,
    created_by_user_id,
    created_at
)
SELECT
    id,
    workspace_id,
    email,
    role,
    token_hash,
    expires_at,
    accepted_at,
    revoked_at,
    created_by_user_id,
    created_at
FROM workspace_invites;

DROP TABLE workspace_invites;
ALTER TABLE workspace_invites_new RENAME TO workspace_invites;

CREATE INDEX idx_workspace_invites_workspace
    ON workspace_invites(workspace_id);
CREATE INDEX idx_workspace_invites_email
    ON workspace_invites(email);

CREATE TABLE application_access_invites_new (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    accepted_at TEXT,
    revoked_at TEXT,
    created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    CONSTRAINT application_access_invites_terminal_state_check
        CHECK (accepted_at IS NULL OR revoked_at IS NULL)
);

INSERT INTO application_access_invites_new (
    id,
    email,
    token_hash,
    expires_at,
    accepted_at,
    revoked_at,
    created_by_user_id,
    created_at
)
SELECT
    id,
    email,
    token_hash,
    expires_at,
    accepted_at,
    revoked_at,
    created_by_user_id,
    created_at
FROM application_access_invites;

DROP TABLE application_access_invites;
ALTER TABLE application_access_invites_new RENAME TO application_access_invites;

CREATE INDEX idx_application_access_invites_email
    ON application_access_invites(email);
CREATE INDEX idx_application_access_invites_created_by
    ON application_access_invites(created_by_user_id);

PRAGMA foreign_keys=ON;
