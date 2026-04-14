-- Phase 1.4/1.14: invoice model extensions and nullable optional fields.

-- New required invoice semantic type.
ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS invoice_type VARCHAR(32);
UPDATE invoices
SET invoice_type = 'vat'
WHERE invoice_type IS NULL;
ALTER TABLE invoices
    ALTER COLUMN invoice_type SET NOT NULL;

-- Correction / linked invoice metadata.
ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS corrected_invoice_number VARCHAR(64),
    ADD COLUMN IF NOT EXISTS correction_reason TEXT,
    ADD COLUMN IF NOT EXISTS original_ksef_number VARCHAR(128),
    ADD COLUMN IF NOT EXISTS advance_payment_date DATE;

-- These are optional in the domain model, so keep DB shape aligned.
ALTER TABLE invoices
    ALTER COLUMN sale_date DROP NOT NULL,
    ALTER COLUMN seller_nip DROP NOT NULL,
    ALTER COLUMN buyer_nip DROP NOT NULL,
    ALTER COLUMN payment_method DROP NOT NULL,
    ALTER COLUMN payment_deadline DROP NOT NULL;

CREATE INDEX IF NOT EXISTS idx_invoices_invoice_type ON invoices(invoice_type);
