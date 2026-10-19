-- 點數批次（D55，docs/04 t-point_lots / contribution_lot_allocations）：錢包內的點數依來源分批，
-- 來源決定能否退現金。balance 仍是 point_wallets 的快取 = Σ point_lots.remaining_points。
-- 所有批次異動都先持有 wallet 列鎖，因此 point_lots 不另行加鎖（見 src/points.rs）。
CREATE TYPE point_lot_source AS ENUM ('topup', 'gift_card', 'reward_bonus', 'adjustment');
ALTER TYPE ledger_entry_type ADD VALUE IF NOT EXISTS 'cash_refund';   -- 現金退款申請扣點（B 部分使用）

CREATE TABLE point_lots (
  id               uuid PRIMARY KEY DEFAULT uuid_v7(),
  wallet_id        uuid             NOT NULL REFERENCES point_wallets(id) ON DELETE RESTRICT,
  source           point_lot_source NOT NULL,
  source_ref_id    uuid,            -- topups.id / gift_cards.id / reward_offers.id；舊資料批次 = wallet id
  original_points  bigint           NOT NULL CHECK (original_points > 0),
  remaining_points bigint           NOT NULL,
  cash_refundable  boolean          NOT NULL,
  expires_at       timestamptz,
  created_at       timestamptz      NOT NULL DEFAULT now(),
  CONSTRAINT point_lots_remaining_range CHECK (remaining_points >= 0 AND remaining_points <= original_points),
  CONSTRAINT point_lots_source_rules CHECK (CASE source
      WHEN 'topup'        THEN cash_refundable AND expires_at IS NULL
      WHEN 'gift_card'    THEN NOT cash_refundable AND expires_at IS NULL
      WHEN 'reward_bonus' THEN NOT cash_refundable AND expires_at IS NOT NULL
      ELSE NOT cash_refundable END)                                   -- adjustment：人工發放 / 舊資料，不可退現金
);
CREATE INDEX point_lots_wallet_idx ON point_lots (wallet_id, source, expires_at) WHERE remaining_points > 0;
CREATE INDEX point_lots_expiry_idx ON point_lots (expires_at) WHERE source = 'reward_bonus' AND remaining_points > 0;
-- 同一來源只能入帳一次（儲值 webhook 重送、禮物卡重複兌換）
CREATE UNIQUE INDEX point_lots_source_uk ON point_lots (source, source_ref_id) WHERE source IN ('topup', 'gift_card');

-- 認捐扣點分配：這筆認捐扣自哪些批次；released / 差額退回時退回原批次
CREATE TABLE contribution_lot_allocations (
  contribution_id uuid   NOT NULL REFERENCES contributions(id) ON DELETE RESTRICT,
  lot_id          uuid   NOT NULL REFERENCES point_lots(id) ON DELETE RESTRICT,
  points          bigint NOT NULL CHECK (points > 0),
  refunded_points bigint NOT NULL DEFAULT 0,
  PRIMARY KEY (contribution_id, lot_id),
  CONSTRAINT allocations_refunded_range CHECK (refunded_points >= 0 AND refunded_points <= points)
);
CREATE INDEX contribution_lot_allocations_lot_idx ON contribution_lot_allocations (lot_id);

-- ledger 帶批次；歷史資料無 lot_id（帳本只增不改）
ALTER TABLE point_ledger ADD COLUMN lot_id uuid REFERENCES point_lots(id) ON DELETE RESTRICT;
CREATE INDEX point_ledger_lot_idx ON point_ledger (lot_id) WHERE lot_id IS NOT NULL;
-- 一筆 pledge / release 會拆成每批次一列：唯一鍵加入 lot_id（冪等語意不變：同認捐、同類型、同批次只能記一次）
DROP INDEX point_ledger_ref_uk;
CREATE UNIQUE INDEX point_ledger_ref_uk ON point_ledger (wallet_id, entry_type, ref_type, ref_id, lot_id)
  WHERE ref_type IN ('contribution', 'cash_refund_request', 'gift_card');
CREATE UNIQUE INDEX point_ledger_ref_legacy_uk ON point_ledger (wallet_id, entry_type, ref_type, ref_id)
  WHERE ref_type = 'contribution' AND lot_id IS NULL;               -- 舊列（無批次）維持原本的防重複

-- ---------- 回填 ----------
-- 每個有歷史餘額或未結認捐（pledged / captured 尚未退回的部分）的錢包建一個舊資料批次：
-- remaining = balance（Σ lots = balance）；original = balance + 認捐中點數，退回時才不會超過 original。
-- 舊批次不可退現金；其 ledger 歷史沒有 lot_id，所以只對帳錢包層級（Σ lots = balance）。
WITH held AS (
  SELECT wallet_id, sum(points - refunded_points)::bigint AS pts
    FROM contributions WHERE status IN ('pledged', 'captured') GROUP BY wallet_id
)
INSERT INTO point_lots (wallet_id, source, source_ref_id, original_points, remaining_points, cash_refundable)
SELECT w.id, 'adjustment', w.id, w.balance + COALESCE(h.pts, 0), w.balance, false
  FROM point_wallets w LEFT JOIN held h ON h.wallet_id = w.id
 WHERE w.balance + COALESCE(h.pts, 0) > 0;

INSERT INTO contribution_lot_allocations (contribution_id, lot_id, points, refunded_points)
SELECT c.id, l.id, c.points, c.refunded_points
  FROM contributions c JOIN point_lots l ON l.wallet_id = c.wallet_id AND l.source = 'adjustment' AND l.source_ref_id = c.wallet_id
 WHERE c.status IN ('pledged', 'captured');
