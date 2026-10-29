-- 生日清單通知（D65）：壽星在一份清單開 birthday_notify，生日前 7 天通知看得到該清單的好友；前 21 天提醒壽星本人開清單。
ALTER TABLE wishlists ADD COLUMN birthday_notify boolean NOT NULL DEFAULT false;
-- 每位擁有者最多一份（開第二份時 API 會先關掉舊的）
CREATE UNIQUE INDEX wishlists_birthday_notify_uk ON wishlists (owner_id) WHERE birthday_notify AND deleted_at IS NULL;

-- 記錄「哪一年的生日」已寄過，條件式 UPDATE 保證每年只排一次（多實例安全）
ALTER TABLE users
  ADD COLUMN birthday_notified_year smallint,   -- 朋友通知（7 天前）
  ADD COLUMN birthday_nudged_year   smallint;   -- 壽星提醒（21 天前）

-- 上線當下不補寄壽星提醒：生日已在 21 天窗口內（台北日期）的既有用戶，今年直接記為已提醒；之後才進入窗口的照常寄。
-- 下次生日的算法同 friends::days_until（非閏年 02-29 當 02-28）。朋友通知不必處理：上線時還沒有清單開啟 birthday_notify。
-- 用「當月 1 日 + (日-1)」避免 make_date 遇到不合法日期報錯；跑出月份不符的（如 02-30）直接略過。
WITH c AS (
  SELECT u.id, t.today, v.y, substr(u.birthday, 1, 2)::int AS m,
         CASE WHEN u.birthday = '02-29' THEN make_date(v.y, 3, 1) - 1   -- 閏年 02-29、平年 02-28
              ELSE make_date(v.y, substr(u.birthday, 1, 2)::int, 1) + (substr(u.birthday, 4, 2)::int - 1) END AS on_day
    FROM users u
   CROSS JOIN (SELECT (now() AT TIME ZONE 'Asia/Taipei')::date AS today) t
   CROSS JOIN LATERAL (VALUES (extract(year FROM t.today)::int), (extract(year FROM t.today)::int + 1)) v(y)
   WHERE u.birthday IS NOT NULL
),
n AS (
  SELECT DISTINCT ON (id) id, y, on_day - today AS days FROM c
   WHERE on_day >= today AND extract(month FROM on_day) = m
   ORDER BY id, on_day
)
UPDATE users u SET birthday_nudged_year = n.y FROM n WHERE u.id = n.id AND n.days <= 21;
