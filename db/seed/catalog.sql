-- 靈感頁模擬資料（正式環境也執行，見 README；真實商品上架後於後台下架）：10 個品牌 + 12 件 Wishly 精選（brand_id 依品牌 slug 對應）。可重複執行。
INSERT INTO brands (slug, name, active, sort_order) VALUES
  ('jo-malone-london','Jo Malone London',true,1), ('hario','HARIO',true,2), ('aden-anais','Aden + Anais',true,3),
  ('osim','OSIM',true,4), ('stanley','Stanley',true,5), ('hape','Hape',true,6), ('le-creuset','Le Creuset',true,7),
  ('pendleton','Pendleton',true,8), ('lamy','LAMY',true,9), ('xiaomi','Xiaomi',true,10)
ON CONFLICT DO NOTHING;

INSERT INTO catalog_products (ref, brand_id, title, spec, note, price_amount, product_url, recipients, occasions, active, sort_order)
SELECT v.ref, (SELECT id FROM brands WHERE slug = v.brand), v.title, v.spec, v.note, v.price,
       'https://www.momoshop.com.tw/search/searchShop.jsp?keyword=' || v.title, v.rec, v.occ, true, v.ord
FROM (VALUES
  ('p01','jo-malone-london','香氛蠟燭禮盒','30g 三入','溫柔木質香調，送禮不出錯。',3200,'{partner,friend}'::text[],'{birthday,holiday}'::text[],1),
  ('p02','hario','手沖咖啡濾杯組','V60 02 套組','入門手沖一組到位。',1280,'{friend,elder}','{housewarming,birthday}',2),
  ('p03','aden-anais','純棉嬰兒包巾','四入組','透氣輕薄，四季都能用。',1450,'{baby}','{full_month}',3),
  ('p04','osim','電動按摩肩頸枕','uNeck 2','久坐或長輩日常放鬆。',3680,'{elder,partner}','{holiday,birthday}',4),
  ('p05','stanley','保溫杯','Quencher 887ml','大容量，通勤運動兩相宜。',1590,'{friend,partner}','{birthday,holiday}',5),
  ('p06','hape','木質積木玩具','彩虹積木 50 件','無毒塗料，啟發手眼協調。',890,'{baby}','{full_month,birthday}',6),
  ('p07','le-creuset','鑄鐵琺瑯鍋','20cm','新居開伙的經典選擇。',6800,'{partner,friend}','{housewarming}',7),
  ('p08',NULL,'香草植栽組','盆栽三件組','陽台就能種，實用不佔空間。',590,'{friend,elder}','{housewarming}',8),
  ('p09','pendleton','純羊毛圍巾',NULL,'冬日保暖，顏色百搭。',2480,'{partner,elder}','{holiday}',9),
  ('p10','lamy','筆記本與鋼筆禮盒','Safari 鋼筆組','文青與上班族都喜歡。',1380,'{friend}','{birthday,holiday}',10),
  ('p11','xiaomi','智慧體重計','Body Composition S400','連動手機，關心健康。',790,'{elder,friend}','{holiday}',11),
  ('p12',NULL,'彌月禮盒','手工蛋黃酥 12 入','傳統彌月分享首選。',980,'{baby,friend}','{full_month}',12)
) AS v(ref, brand, title, spec, note, price, rec, occ, ord)
ON CONFLICT DO NOTHING;
