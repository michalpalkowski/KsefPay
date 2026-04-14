-- payment_method and payment_deadline should be nullable:
-- fetched invoices from KSeF may not have payment information.
-- Existing rows with payment_method=0 are converted to NULL.

ALTER TABLE invoices ALTER COLUMN payment_method DROP NOT NULL;
ALTER TABLE invoices ALTER COLUMN payment_deadline DROP NOT NULL;

UPDATE invoices SET payment_method = NULL WHERE payment_method = 0;
