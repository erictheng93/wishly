-- 儲值（骨架）：1 點 = NT$1。金流商（綠界 / 藍新）尚未接入，webhook 驗簽後走 points::post 入帳。
-- 點數不可提領、不可轉讓（見 0004）；單一錢包上限 NT$50,000（電子支付機構管理條例的儲值上限）由應用層檢查。
ALTER TYPE ledger_entry_type ADD VALUE IF NOT EXISTS 'topup';

CREATE TABLE topups (
  id                uuid PRIMARY KEY DEFAULT uuid_v7(),
  user_id           uuid        NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  merchant_trade_no text        NOT NULL UNIQUE,                 -- 我方訂單編號（給金流商）
  provider          text        NOT NULL,
  method            text        NOT NULL DEFAULT 'card',         -- card | cvs_code | cvs_barcode | atm
  amount            bigint      NOT NULL CHECK (amount > 0),     -- NT$ = 點數
  status            text        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid', 'expired', 'failed')),
  provider_trade_no text,                                        -- 金流商交易編號（對帳）
  expires_at        timestamptz NOT NULL,                        -- 超商代碼 / 虛擬帳號繳費期限
  paid_at           timestamptz,
  created_at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX topups_user_idx ON topups (user_id, created_at DESC);
CREATE UNIQUE INDEX topups_provider_trade_uk ON topups (provider, provider_trade_no) WHERE provider_trade_no IS NOT NULL;

-- 防重複入帳：同一筆儲值只能記一次 topup
CREATE UNIQUE INDEX point_ledger_topup_uk ON point_ledger (wallet_id, entry_type, ref_id) WHERE ref_type = 'topup';
