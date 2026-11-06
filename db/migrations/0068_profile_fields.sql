-- 個人頁純量欄位：送禮偏好、興趣、年齡、封面；各欄位自有可見性
CREATE TYPE profile_vis AS ENUM ('public', 'friends', 'close', 'private');

ALTER TABLE users
  ADD COLUMN gift_likes       text[] NOT NULL DEFAULT '{}',
  ADD COLUMN gift_dislikes    text[] NOT NULL DEFAULT '{}',
  ADD COLUMN gift_prefs_vis   profile_vis NOT NULL DEFAULT 'friends',
  ADD COLUMN interests        text[] NOT NULL DEFAULT '{}',
  ADD COLUMN interests_vis    profile_vis NOT NULL DEFAULT 'public',
  ADD COLUMN birth_year       smallint CHECK (birth_year BETWEEN 1900 AND 2100),
  ADD COLUMN age_vis          profile_vis NOT NULL DEFAULT 'private' CHECK (age_vis <> 'public'),
  ADD COLUMN home_address_vis profile_vis NOT NULL DEFAULT 'private' CHECK (home_address_vis <> 'public'),
  ADD COLUMN cover_preset     text CHECK (cover_preset IN ('birthday','wedding','baby','housewarming','graduation','holiday','travel','gift')),
  ADD COLUMN cover_image_key  text,
  ADD CONSTRAINT users_cover_one CHECK (cover_preset IS NULL OR cover_image_key IS NULL);
