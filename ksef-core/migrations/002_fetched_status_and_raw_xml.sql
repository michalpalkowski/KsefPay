-- Add 'fetched' status for invoices downloaded from KSeF
ALTER TABLE invoices DROP CONSTRAINT IF EXISTS invoices_status_check;
ALTER TABLE invoices ADD CONSTRAINT invoices_status_check
    CHECK (status IN ('draft', 'queued', 'submitted', 'accepted', 'rejected', 'failed', 'fetched'));

-- Store original FA(3) XML from KSeF (audit trail)
ALTER TABLE invoices ADD COLUMN IF NOT EXISTS raw_xml TEXT;
