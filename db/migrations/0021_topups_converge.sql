-- 儲值設計收斂（D54）：把 0020 的精簡 topups 收斂到 docs/04 的完整設計（階段 1：topups / invoices）。
-- 0020 的功能預設關閉，正式環境沒有資料；仍以 UPDATE 回填，避免開發資料庫遺失。
CREATE TYPE topup_method         AS ENUM ('card', 'line_pay', 'apple_pay', 'cvs_code', 'cvs_barcode', 'atm');
CREATE TYPE payment_provider     AS ENUM ('tappay', 'ecpay');
CREATE TYPE topup_status         AS ENUM ('pending', 'paid', 'failed', 'expired', 'refunded', 'disputed', 'charged_back');
CREATE TYPE invoice_source       AS ENUM ('topup', 'gift_card_order', 'gift_card_batch', 'service_fee');
CREATE TYPE invoice_status       AS ENUM ('pending', 'issued', 'void', 'allowance');
CREATE TYPE invoice_carrier_type AS ENUM ('member', 'mobile_barcode', 'citizen_cert', 'donate', 'b2b');

CREATE TABLE invoices (
  id             uuid PRIMARY KEY DEFAULT uuid_v7(),
  source_type    invoice_source       NOT NULL,
  source_id      uuid                 NOT NULL,
  invoice_no     text                 UNIQUE,
  amount         bigint               NOT NULL CHECK (amount > 0),
  tax_type       text                 NOT NULL DEFAULT 'taxable' CHECK (tax_type IN ('taxable', 'zero_rate', 'tax_free')),
  status         invoice_status       NOT NULL DEFAULT 'pending',
  carrier_type   invoice_carrier_type NOT NULL DEFAULT 'member',
  carrier_id     text,
  donate_code    text,
  buyer_tax_id   char(8),
  buyer_name     text,
  issued_at      timestamptz,
  voided_at      timestamptz,
  created_at     timestamptz          NOT NULL DEFAULT now(),
  updated_at     timestamptz          NOT NULL DEFAULT now(),
  CONSTRAINT invoices_carrier_fields CHECK (
    (carrier_type <> 'mobile_barcode' OR carrier_id IS NOT NULL) AND
    (carrier_type <> 'citizen_cert'   OR carrier_id IS NOT NULL) AND
    (carrier_type <> 'donate'         OR donate_code IS NOT NULL) AND
    (carrier_type <> 'b2b'            OR (buyer_tax_id IS NOT NULL AND buyer_tax_id ~ '^[0-9]{8}$'))),
  CONSTRAINT invoices_issued_has_no CHECK (status NOT IN ('issued', 'allowance') OR (invoice_no IS NOT NULL AND issued_at IS NOT NULL))
);
CREATE UNIQUE INDEX invoices_source_active_uk ON invoices (source_type, source_id) WHERE status IN ('pending', 'issued');
CREATE INDEX invoices_source_idx ON invoices (source_type, source_id);

-- topups：user_id → wallet_id；provider_trade_no → provider_txn_id；text 欄位改 enum
ALTER TABLE topups ADD COLUMN wallet_id uuid REFERENCES point_wallets(id) ON DELETE RESTRICT;
INSERT INTO point_wallets (user_id) SELECT DISTINCT user_id FROM topups ON CONFLICT (user_id) DO NOTHING;
UPDATE topups t SET wallet_id = w.id FROM point_wallets w WHERE w.user_id = t.user_id;
ALTER TABLE topups ALTER COLUMN wallet_id SET NOT NULL;
DROP INDEX topups_user_idx;
ALTER TABLE topups DROP COLUMN user_id;
ALTER TABLE topups RENAME COLUMN provider_trade_no TO provider_txn_id;
DROP INDEX topups_provider_trade_uk;
ALTER TABLE topups ADD CONSTRAINT topups_provider_txn_uk UNIQUE (provider_txn_id);

ALTER TABLE topups DROP CONSTRAINT topups_status_check;
ALTER TABLE topups ALTER COLUMN status DROP DEFAULT;
ALTER TABLE topups ALTER COLUMN method DROP DEFAULT;
ALTER TABLE topups ALTER COLUMN method   TYPE topup_method     USING method::topup_method;
ALTER TABLE topups ALTER COLUMN provider TYPE payment_provider USING provider::payment_provider;
ALTER TABLE topups ALTER COLUMN status   TYPE topup_status     USING status::topup_status;
ALTER TABLE topups ALTER COLUMN status SET DEFAULT 'pending';

ALTER TABLE topups
  ADD COLUMN is_auto        boolean     NOT NULL DEFAULT false,
  ADD COLUMN invoice_id     uuid        REFERENCES invoices(id) ON DELETE RESTRICT,
  ADD COLUMN failure_reason text,
  ADD COLUMN updated_at     timestamptz NOT NULL DEFAULT now(),
  ADD CONSTRAINT topups_paid_has_txn CHECK (status <> 'paid' OR (provider_txn_id IS NOT NULL AND paid_at IS NOT NULL));
CREATE INDEX topups_wallet_idx ON topups (wallet_id, created_at DESC);
CREATE INDEX topups_pending_idx ON topups (created_at) WHERE status = 'pending';
