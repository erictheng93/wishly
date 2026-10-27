-- 拒付後退回並抵銷欠款的點數（pledge 撤回 / 現金退款被拒絕等）；未抵銷欠款 = amount - recovered_points。
ALTER TABLE wallet_debts
  ADD COLUMN recovered_points bigint NOT NULL DEFAULT 0,
  ADD CONSTRAINT wallet_debts_recovered_range CHECK (recovered_points >= 0 AND recovered_points <= amount);
