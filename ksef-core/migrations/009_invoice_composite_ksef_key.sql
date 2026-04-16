-- An invoice can appear in multiple NIP accounts: once as Outgoing for the issuer,
-- and once as Incoming for the recipient. The old UNIQUE(ksef_number) constraint
-- forced a single owner, causing the issuer's copy to be overwritten when the
-- recipient fetched the same invoice.
--
-- Fix: make the uniqueness scope per (ksef_number, nip_account_id) so each
-- account holds its own tenant-isolated copy of the invoice.

ALTER TABLE invoices DROP CONSTRAINT IF EXISTS invoices_ksef_number_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'invoices_ksef_number_nip_account_uniq'
          AND conrelid = 'invoices'::regclass
    ) THEN
        ALTER TABLE invoices
            ADD CONSTRAINT invoices_ksef_number_nip_account_uniq
            UNIQUE (ksef_number, nip_account_id);
    END IF;
END
$$;
