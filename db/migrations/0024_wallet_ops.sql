-- 錢包後續（D55）：現金退款申請、拒付欠款、自動儲值設定（docs/04 t-cash_refund_requests / t-wallet_debts / t-wallet_auto_topups）。
-- 0023 保留給儲值階段 1。自動儲值的觸發 job 與扣款需要金流授權，尚未實作，這裡只有設定表。
CREATE TYPE refund_request_status AS ENUM ('requested', 'processing', 'completed', 'rejected');
CREATE TYPE wallet_debt_status    AS ENUM ('open', 'settled', 'written_off');

CREATE TABLE cash_refund_requests (
  id              uuid PRIMARY KEY DEFAULT uuid_v7(),
  wallet_id       uuid                  NOT NULL REFERENCES point_wallets(id) ON DELETE RESTRICT,
  amount          bigint                NOT NULL CHECK (amount > 0),
  status          refund_request_status NOT NULL DEFAULT 'requested',
  lot_allocations jsonb                 NOT NULL DEFAULT '[]'::jsonb,   -- [{lot_id, points}]：扣自哪些 cash_refundable 批次；rejected 時據此退回原批次
  destination_enc bytea,                -- 退款帳戶（應用層 AEAD 加密）；原路退回時為 NULL（尚未實作收集）
  reject_reason   text,
  processed_by    uuid                  REFERENCES users(id) ON DELETE SET NULL,
  completed_at    timestamptz,
  created_at      timestamptz           NOT NULL DEFAULT now(),
  updated_at      timestamptz           NOT NULL DEFAULT now(),
  CONSTRAINT cash_refund_rejected_has_reason CHECK (status <> 'rejected' OR reject_reason IS NOT NULL)
);
CREATE INDEX cash_refund_requests_wallet_idx ON cash_refund_requests (wallet_id, created_at DESC);
CREATE INDEX cash_refund_requests_open_idx ON cash_refund_requests (created_at) WHERE status IN ('requested', 'processing');

CREATE TABLE wallet_debts (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  wallet_id   uuid               NOT NULL REFERENCES point_wallets(id) ON DELETE RESTRICT,
  topup_id    uuid               NOT NULL REFERENCES topups(id) ON DELETE RESTRICT,
  amount      bigint             NOT NULL CHECK (amount > 0),       -- 拒付時已被花掉、無法扣回的點數價值；balance CHECK >= 0 不變
  status      wallet_debt_status NOT NULL DEFAULT 'open',
  note        text,
  resolved_by uuid               REFERENCES users(id) ON DELETE SET NULL,
  resolved_at timestamptz,
  created_at  timestamptz        NOT NULL DEFAULT now(),
  updated_at  timestamptz        NOT NULL DEFAULT now(),
  UNIQUE (topup_id),                                                -- 同一筆拒付只記一次欠款（重送冪等）
  CONSTRAINT wallet_debts_resolved_consistent CHECK ((status = 'open') = (resolved_at IS NULL))
);
CREATE INDEX wallet_debts_wallet_idx ON wallet_debts (wallet_id, status);

CREATE TABLE wallet_auto_topups (
  user_id            uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  enabled            boolean          NOT NULL DEFAULT false,
  threshold_points   bigint           NOT NULL DEFAULT 500 CHECK (threshold_points >= 0),
  amount             bigint           NOT NULL DEFAULT 1000 CHECK (amount > 0),
  method             topup_method     NOT NULL DEFAULT 'card' CHECK (method = 'card'),
  provider           payment_provider NOT NULL DEFAULT 'tappay',
  card_token_ref     text,            -- 金流商 card token 參照，絕不存卡號（綁卡需金流商，尚未接入）
  card_last4         char(4),
  monthly_cap_amount bigint           NOT NULL DEFAULT 10000 CHECK (monthly_cap_amount > 0),
  created_at         timestamptz      NOT NULL DEFAULT now(),
  updated_at         timestamptz      NOT NULL DEFAULT now(),
  CONSTRAINT auto_topup_enabled_has_card CHECK (NOT enabled OR card_token_ref IS NOT NULL),
  CONSTRAINT auto_topup_amount_within_cap CHECK (amount <= monthly_cap_amount)
);

CREATE TRIGGER cash_refund_requests_set_updated_at BEFORE UPDATE ON cash_refund_requests FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER wallet_debts_set_updated_at         BEFORE UPDATE ON wallet_debts         FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER wallet_auto_topups_set_updated_at   BEFORE UPDATE ON wallet_auto_topups   FOR EACH ROW EXECUTE FUNCTION set_updated_at();
