ALTER TABLE user_nip_access
    ADD COLUMN can_manage_credentials INTEGER NOT NULL DEFAULT 1;
