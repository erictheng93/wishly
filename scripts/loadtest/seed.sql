-- 壓測資料（只給本機 / 壓測環境，切勿在正式 DB 執行）
-- psql "$DATABASE_URL" -v users=2000 -f scripts/loadtest/seed.sql
-- 產生：:users 位使用者（登入權杖 'lt-<n>'，sessions 只存 SHA-256）、每人一份 public + 一份 friends 清單（各 10 品項）、
-- 每位使用者與下一位互為好友（讓 friends 清單有人看得到），以及部分既有捐助與站內通知。
\set ON_ERROR_STOP on
\if :{?users}
\else
  \set users 2000
\endif

BEGIN;
INSERT INTO users (display_name, email)
SELECT 'lt user ' || n, 'lt' || n || '@loadtest.invalid' FROM generate_series(1, :users) n;

CREATE TEMP TABLE lt_u AS
SELECT id, (regexp_replace(display_name, '\D', '', 'g'))::int AS n FROM users WHERE email LIKE 'lt%@loadtest.invalid';
CREATE INDEX ON lt_u (n);

INSERT INTO sessions (user_id, token_hash, expires_at, user_agent)
SELECT id, sha256(convert_to('lt-' || n, 'UTF8')), now() + interval '30 days', 'k6' FROM lt_u;

INSERT INTO wishlists (owner_id, type, status, visibility, slug, title, description, event_date)
SELECT u.id, 'registry', 'active', v.vis::visibility,
       v.p || lpad(u.n::text, 8, '0'), 'Load test ' || v.vis || ' #' || u.n, '壓測用清單，生日快樂！', current_date + 30
FROM lt_u u CROSS JOIN (VALUES ('pu', 'public'), ('fr', 'friends')) v(p, vis);

INSERT INTO wishlist_items (wishlist_id, title, brand, spec, priority, qty_needed, qty_claimed, unit_price_amount, sort_order)
SELECT w.id, '品項 ' || k, 'Brand', '規格說明', (ARRAY['high','medium','low'])[1 + k % 3]::item_priority,
       1 + k % 3, 0, 300 + k * 100, k
FROM wishlists w JOIN lt_u u ON u.id = w.owner_id CROSS JOIN generate_series(1, 10) k;
COMMIT;

-- 好友：n 與 n+1（環狀），雙向
BEGIN;
INSERT INTO friendships (user_a, user_b)
SELECT LEAST(a.id, b.id), GREATEST(a.id, b.id) FROM lt_u a JOIN lt_u b ON b.n = a.n % :users + 1
ON CONFLICT DO NOTHING;
COMMIT;

ANALYZE;
