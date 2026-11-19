ALTER TABLE claims
    ADD COLUMN thank_you text CHECK (char_length(thank_you) BETWEEN 1 AND 500),
    ADD COLUMN thank_you_muted boolean NOT NULL DEFAULT false;

-- Only an explicitly supplied claim reference associates a receipt with its donor.
ALTER TABLE receipts ADD COLUMN claim_id uuid REFERENCES claims(id) ON DELETE SET NULL;
CREATE UNIQUE INDEX receipts_active_claim_idx ON receipts (claim_id)
    WHERE claim_id IS NOT NULL AND voided_at IS NULL;
