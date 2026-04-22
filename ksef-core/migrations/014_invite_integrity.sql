DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'workspace_invites_terminal_state_check'
    ) THEN
        ALTER TABLE workspace_invites
            ADD CONSTRAINT workspace_invites_terminal_state_check
            CHECK (accepted_at IS NULL OR revoked_at IS NULL);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'application_access_invites_terminal_state_check'
    ) THEN
        ALTER TABLE application_access_invites
            ADD CONSTRAINT application_access_invites_terminal_state_check
            CHECK (accepted_at IS NULL OR revoked_at IS NULL);
    END IF;
END $$;
