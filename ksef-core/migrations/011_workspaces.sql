CREATE TABLE IF NOT EXISTS workspaces (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS workspace_memberships (
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'operator', 'read_only')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'invited', 'revoked')),
    can_manage_members BOOLEAN NOT NULL DEFAULT FALSE,
    can_manage_nips BOOLEAN NOT NULL DEFAULT FALSE,
    can_manage_credentials BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_memberships_user
    ON workspace_memberships(user_id);

CREATE TABLE IF NOT EXISTS workspace_nip_accounts (
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    nip_account_id UUID NOT NULL REFERENCES nip_accounts(id) ON DELETE CASCADE,
    ownership_type TEXT NOT NULL DEFAULT 'workspace_owned'
        CHECK (ownership_type IN ('workspace_owned', 'delegated', 'migrated_legacy')),
    attached_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, nip_account_id),
    UNIQUE (nip_account_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_nip_accounts_workspace
    ON workspace_nip_accounts(workspace_id);

CREATE TABLE IF NOT EXISTS workspace_invites (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'operator', 'read_only')),
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workspace_invites_workspace
    ON workspace_invites(workspace_id);

CREATE INDEX IF NOT EXISTS idx_workspace_invites_email
    ON workspace_invites(email);

DO $$
BEGIN
    IF to_regclass('public.user_nip_access') IS NOT NULL THEN
        WITH ranked_legacy_access AS (
            SELECT
                una.nip_account_id,
                una.user_id,
                u.email,
                una.can_manage_credentials,
                ROW_NUMBER() OVER (
                    PARTITION BY una.nip_account_id
                    ORDER BY una.can_manage_credentials DESC, u.email, una.user_id
                ) AS rn
            FROM user_nip_access una
            INNER JOIN users u ON u.id = una.user_id
        ),
        anchor_workspaces AS (
            SELECT DISTINCT
                user_id AS workspace_id,
                email
            FROM ranked_legacy_access
            WHERE rn = 1
        )
        INSERT INTO workspaces (id, slug, display_name, created_by_user_id, created_at, updated_at)
        SELECT
            workspace_id,
            CONCAT(SPLIT_PART(email, '@', 1), '-', LEFT(workspace_id::text, 8)),
            CONCAT(SPLIT_PART(email, '@', 1), ' workspace'),
            workspace_id,
            NOW(),
            NOW()
        FROM anchor_workspaces
        ON CONFLICT (id) DO NOTHING;
    END IF;
END $$;

DO $$
BEGIN
    IF to_regclass('public.user_nip_access') IS NOT NULL THEN
        WITH ranked_legacy_access AS (
            SELECT
                una.nip_account_id,
                una.user_id,
                u.email,
                una.can_manage_credentials,
                ROW_NUMBER() OVER (
                    PARTITION BY una.nip_account_id
                    ORDER BY una.can_manage_credentials DESC, u.email, una.user_id
                ) AS rn
            FROM user_nip_access una
            INNER JOIN users u ON u.id = una.user_id
        ),
        anchor_map AS (
            SELECT
                nip_account_id,
                user_id AS workspace_id
            FROM ranked_legacy_access
            WHERE rn = 1
        )
        INSERT INTO workspace_nip_accounts (
            workspace_id,
            nip_account_id,
            ownership_type,
            attached_by_user_id,
            created_at
        )
        SELECT
            workspace_id,
            nip_account_id,
            'migrated_legacy',
            workspace_id,
            NOW()
        FROM anchor_map
        ON CONFLICT (workspace_id, nip_account_id) DO NOTHING;
    END IF;
END $$;

DO $$
BEGIN
    IF to_regclass('public.user_nip_access') IS NOT NULL THEN
        WITH ranked_legacy_access AS (
            SELECT
                una.nip_account_id,
                una.user_id,
                u.email,
                una.can_manage_credentials,
                ROW_NUMBER() OVER (
                    PARTITION BY una.nip_account_id
                    ORDER BY una.can_manage_credentials DESC, u.email, una.user_id
                ) AS rn
            FROM user_nip_access una
            INNER JOIN users u ON u.id = una.user_id
        ),
        anchor_map AS (
            SELECT
                nip_account_id,
                user_id AS workspace_id
            FROM ranked_legacy_access
            WHERE rn = 1
        ),
        role_rows AS (
            SELECT
                am.workspace_id,
                una.user_id,
                CASE
                    WHEN am.workspace_id = una.user_id THEN 3
                    WHEN una.can_manage_credentials THEN 2
                    ELSE 1
                END AS role_rank
            FROM user_nip_access una
            INNER JOIN anchor_map am ON am.nip_account_id = una.nip_account_id
        ),
        aggregated_roles AS (
            SELECT
                workspace_id,
                user_id,
                MAX(role_rank) AS role_rank
            FROM role_rows
            GROUP BY workspace_id, user_id
        )
        INSERT INTO workspace_memberships (
            workspace_id,
            user_id,
            role,
            status,
            can_manage_members,
            can_manage_nips,
            can_manage_credentials,
            created_at,
            updated_at
        )
        SELECT
            workspace_id,
            user_id,
            CASE role_rank
                WHEN 3 THEN 'owner'
                WHEN 2 THEN 'admin'
                ELSE 'operator'
            END,
            'active',
            role_rank >= 2,
            role_rank >= 2,
            role_rank >= 2,
            NOW(),
            NOW()
        FROM aggregated_roles
        ON CONFLICT (workspace_id, user_id) DO NOTHING;
    END IF;
END $$;
