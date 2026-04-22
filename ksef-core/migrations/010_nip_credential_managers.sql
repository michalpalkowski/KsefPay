ALTER TABLE user_nip_access
    ADD COLUMN IF NOT EXISTS can_manage_credentials BOOLEAN NOT NULL DEFAULT TRUE;
