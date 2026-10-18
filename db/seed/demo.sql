-- 範例清單：landing 的「看範例」連結（/s/demobirth0 等）走真實 API，取代僅 deploy:demo 才有的假資料。
-- 不是 migration（會污染測試與統計）；部署後手動執行一次：psql "$DATABASE_URL" -f db/seed/demo.sql（可重複執行：已存在則只確保為 closed）。
-- 清單一律 closed：可瀏覽、但不接受認捐，避免真實使用者對示範人物送出認捐。
-- 擁有者為無 email 的示範帳號（無法登入）；品項只用數量制且 qty_claimed 一律 0（不造假認領），不寫點數。
-- 探索頁排除 slug 'demo%'（見 public.rs），標題帶「【範例】」，避免被當成真人清單。
-- slug 須符合 wishlists_slug_format（10 碼英數），所以不是 "demo"。
WITH o AS (
  INSERT INTO users (display_name)
  SELECT n FROM (VALUES ('小安'), ('Mia'), ('宇軒 & 佩佩')) AS v(n)
  WHERE NOT EXISTS (SELECT 1 FROM wishlists WHERE slug = 'demobirth0')
  RETURNING id, display_name
), w AS (
  INSERT INTO wishlists (owner_id, type, status, visibility, slug, title, description, event_date, closed_at)
  SELECT o.id, 'registry', 'closed', 'public', v.slug, v.title, v.descr, v.ev::date, now()
  FROM o JOIN (VALUES
    ('小安', 'demobirth0', '【範例】小安的生日心願', '11 月 8 日生日，朋友們幫我認領一樣就好', '2026-11-08'),
    ('Mia', 'demogift00', '【範例】Mia 的聖誕心願', '不知道送什麼的話，看這裡就好，捐助後我會自己標記收到', '2026-12-25'),
    ('宇軒 & 佩佩', 'demowedd00', '【範例】宇軒 & 佩佩的新家清單', '1 月 16 日結婚，新家還缺這些，謝謝大家的祝福', '2027-01-16')
  ) AS v(owner, slug, title, descr, ev) ON v.owner = o.display_name
  RETURNING id, slug
)
INSERT INTO wishlist_items (wishlist_id, title, brand, spec, priority, qty_needed, qty_claimed, unit_price_amount, sort_order)
SELECT w.id, i.title, i.brand, i.spec, i.pri::item_priority, i.need, i.got, i.price, i.ord
FROM w JOIN (VALUES
  ('demobirth0', '手沖咖啡組', 'Hario', '濾杯 + 細口壺 + 磨豆機', 'high', 1, 0, 2400, 1),
  ('demobirth0', '降噪耳機', 'Sony', '黑色', 'high', 1, 0, 8000, 2),
  ('demobirth0', '設計的法則', NULL, '繁體中文版', 'medium', 1, 0, 480, 3),
  ('demobirth0', '香氛蠟燭', 'Jo Malone', '任一款', 'low', 2, 0, 350, 4),
  ('demogift00', '羊毛圍巾', NULL, '燕麥色', 'high', 1, 0, 1200, 1),
  ('demogift00', '保溫瓶 500ml', 'Stanley', '霧黑', 'medium', 1, 0, 900, 2),
  ('demogift00', '襪子禮盒', NULL, 'M 尺寸', 'low', 3, 0, 300, 3),
  ('demogift00', '拍立得相機', 'Instax', 'Mini 12', 'high', 1, 0, 5000, 4),
  ('demowedd00', '氣炸鍋 5L', 'Philips', NULL, 'high', 1, 0, 3200, 1),
  ('demowedd00', '餐盤組（6 入）', NULL, '白色陶瓷', 'medium', 2, 0, 1500, 2),
  ('demowedd00', '雙人床包組', NULL, '灰藍色 6×7 尺', 'medium', 1, 0, 2200, 3),
  ('demowedd00', '掃地機器人', 'Roborock', NULL, 'high', 1, 0, 12000, 4),
  ('demowedd00', '玻璃保鮮盒', NULL, NULL, 'low', 3, 0, 600, 5)
) AS i(slug, title, brand, spec, pri, need, got, price, ord) ON i.slug = w.slug;

-- 先前以 active 建立過的範例清單（重複執行時）一併改為 closed
UPDATE wishlists SET status = 'closed', closed_at = COALESCE(closed_at, now())
 WHERE slug IN ('demobirth0', 'demogift00', 'demowedd00') AND status = 'active';
