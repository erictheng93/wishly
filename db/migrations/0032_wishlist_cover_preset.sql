-- 預設封面：不上傳圖片時改選內建圖（圖檔在前端 /covers/{id}.svg，後端只存 id）
ALTER TABLE wishlists ADD COLUMN cover_preset text
  CHECK (cover_preset IN ('birthday','wedding','baby','housewarming','graduation','holiday','travel','gift'));
