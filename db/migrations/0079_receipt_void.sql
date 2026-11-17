-- 實收更正：作廢原紀錄（不刪列），加總一律排除 voided_at 非空者。
-- voided_by 的 FK 不需帳號刪除額外處理：帳號刪除是匿名化 users 列、不真刪。
ALTER TABLE receipts
  ADD COLUMN voided_at  timestamptz,
  ADD COLUMN voided_by  uuid REFERENCES users(id),
  ADD COLUMN void_reason text CHECK (char_length(void_reason) <= 200);
