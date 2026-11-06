-- 帳號層級的預設收件地址（shipping）與居住地址（home）；欄位級加密同 shipping_addresses
CREATE TABLE user_addresses (
  user_id            uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind               text NOT NULL CHECK (kind IN ('shipping', 'home')),
  recipient_name_enc bytea,
  phone_enc          bytea,
  address_enc        bytea NOT NULL,
  updated_at         timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, kind),
  CHECK (kind = 'home' OR (recipient_name_enc IS NOT NULL AND phone_enc IS NOT NULL))
);
