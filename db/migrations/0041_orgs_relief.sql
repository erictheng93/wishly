-- 組織、救災事件、據點欄位（spec 2026-10-27-org-relief-registration-design）。
-- 應用層會先檢查；這裡的 CHECK / trigger 是 DB 層安全網。

CREATE TYPE org_role AS ENUM ('owner', 'coordinator', 'volunteer');

-- ---------- 組織 ----------
-- 草稿階段只有 legal_name / display_name 必填；送審（非 draft）後其餘欄位都要齊。
CREATE TABLE organizations (
  id                  uuid PRIMARY KEY DEFAULT uuid_v7(),
  slug                char(10)    NOT NULL UNIQUE,
  legal_name          text        NOT NULL CHECK (char_length(legal_name) BETWEEN 1 AND 100),
  display_name        text        NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 50),
  org_type            text        CHECK (org_type IN ('association', 'foundation', 'religious', 'other')),
  tax_id              char(8)     CHECK (tax_id ~ '^[0-9]{8}$'),
  registration_no     text,
  authority           text,
  website             text,
  intro               text        CHECK (char_length(intro) <= 2000),
  logo_key            text,
  rep_name            text,
  contact_name        text,
  contact_title       text,
  contact_phone_enc   bytea,      -- sealed.rs 加密，不對外
  public_email        text,
  public_phone        text,
  verification_status text        NOT NULL DEFAULT 'draft'
                      CHECK (verification_status IN ('draft', 'pending', 'needs_changes', 'verified', 'rejected')),
  review_note         text,
  flagged_fields      text[]      NOT NULL DEFAULT '{}',
  resubmit_after      timestamptz,
  submitted_at        timestamptz,
  verified_at         timestamptz,
  created_by          uuid        NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at          timestamptz NOT NULL DEFAULT now(),
  updated_at          timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT organizations_verified_at CHECK ((verification_status = 'verified') = (verified_at IS NOT NULL)),
  CONSTRAINT organizations_submit_required CHECK (verification_status = 'draft' OR (
    org_type IS NOT NULL AND tax_id IS NOT NULL AND registration_no IS NOT NULL AND authority IS NOT NULL
    AND rep_name IS NOT NULL AND contact_name IS NOT NULL AND contact_phone_enc IS NOT NULL AND public_email IS NOT NULL))
);
-- 同一統編同時只能有一筆「在審 / 已通過」；被拒的不佔位
CREATE UNIQUE INDEX organizations_tax_id_live ON organizations (tax_id)
  WHERE verification_status IN ('pending', 'needs_changes', 'verified');
CREATE UNIQUE INDEX organizations_one_draft ON organizations (created_by) WHERE verification_status = 'draft';
CREATE TRIGGER organizations_set_updated_at BEFORE UPDATE ON organizations FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE org_members (
  org_id      uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id     uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role        org_role    NOT NULL,
  handle      text        CHECK (char_length(handle) <= 30),
  disabled_at timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id, user_id)
);
CREATE INDEX org_members_user_idx ON org_members (user_id);

CREATE TABLE org_invites (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  org_id      uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  email       text        NOT NULL CHECK (email = lower(email)),
  role        org_role    NOT NULL,
  handle      text,
  token_hash  bytea       NOT NULL UNIQUE,
  invited_by  uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at  timestamptz NOT NULL,
  accepted_at timestamptz,
  cancelled_at timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE org_documents (
  id           uuid PRIMARY KEY DEFAULT uuid_v7(),
  org_id       uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  kind         text        NOT NULL CHECK (kind IN ('registration', 'permit')),
  object_key   text        NOT NULL,
  content_type text        NOT NULL,
  size_bytes   int         NOT NULL,
  uploaded_by  uuid        NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE org_permits (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  org_id      uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  permit_no   text        NOT NULL CHECK (char_length(permit_no) BETWEEN 1 AND 60),
  valid_from  date        NOT NULL,
  valid_to    date        NOT NULL,
  document_id uuid        REFERENCES org_documents(id),
  status      text        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
  review_note text,
  created_at  timestamptz NOT NULL DEFAULT now(),
  CHECK (valid_to >= valid_from)
);

-- ---------- 救災事件 ----------
CREATE TABLE relief_events (
  id                uuid PRIMARY KEY DEFAULT uuid_v7(),
  slug              char(10)    NOT NULL UNIQUE,
  name              text        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
  hazard_type       text        NOT NULL CHECK (hazard_type IN ('earthquake', 'typhoon', 'flood', 'fire', 'other')),
  counties          text[]      NOT NULL CHECK (cardinality(counties) >= 1),
  area_note         text        CHECK (char_length(area_note) <= 200),
  occurred_on       date,
  description       text        CHECK (char_length(description) <= 2000),
  created_by_org    uuid        NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  merged_into       uuid        REFERENCES relief_events(id),
  moderation_status moderation_status NOT NULL DEFAULT 'ok',
  moderation_reason text,
  created_at        timestamptz NOT NULL DEFAULT now(),
  updated_at        timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER relief_events_set_updated_at BEFORE UPDATE ON relief_events FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------- wishlists：據點欄位 ----------
ALTER TABLE wishlists
  ADD COLUMN org_id       uuid REFERENCES organizations(id) ON DELETE RESTRICT,
  ADD COLUMN event_id     uuid REFERENCES relief_events(id),
  ADD COLUMN permit_id    uuid REFERENCES org_permits(id),
  ADD COLUMN site_address text,
  ADD COLUMN site_status  text CHECK (site_status IN ('active', 'paused', 'closed'));

-- relief ⇔ 五欄齊全；非 relief 全為 NULL
ALTER TABLE wishlists ADD CONSTRAINT wishlists_relief_site CHECK (
  (type = 'relief') = (org_id IS NOT NULL AND event_id IS NOT NULL AND permit_id IS NOT NULL
                       AND site_address IS NOT NULL AND site_status IS NOT NULL)
  AND (type = 'relief' OR (org_id IS NULL AND event_id IS NULL AND permit_id IS NULL
                           AND site_address IS NULL AND site_status IS NULL)));
ALTER TABLE wishlists ADD CONSTRAINT wishlists_relief_public CHECK (type <> 'relief' OR visibility = 'public');

-- ---------- wishlist_items：物資分類 / 急迫度 ----------
ALTER TABLE wishlist_items
  ADD COLUMN category text CHECK (category IN ('water', 'food', 'medical', 'hygiene', 'shelter', 'clothing', 'baby', 'other')),
  ADD COLUMN urgency  text CHECK (urgency IN ('critical', 'high', 'medium', 'low'));

-- relief 品項 category / urgency 必填，其餘必為 NULL（兩種 funding_mode 皆可）
CREATE FUNCTION items_relief_shape() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE t wishlist_type;
BEGIN
  SELECT type INTO t FROM wishlists WHERE id = NEW.wishlist_id;
  IF t = 'relief' AND (NEW.category IS NULL OR NEW.urgency IS NULL) THEN
    RAISE EXCEPTION 'relief 品項須有 category 與 urgency' USING ERRCODE = 'check_violation';
  ELSIF t <> 'relief' AND (NEW.category IS NOT NULL OR NEW.urgency IS NOT NULL) THEN
    RAISE EXCEPTION 'only relief items take category/urgency' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER items_relief_shape BEFORE INSERT OR UPDATE OF wishlist_id, category, urgency ON wishlist_items
  FOR EACH ROW EXECUTE FUNCTION items_relief_shape();

-- relief 據點上線：組織須 verified、許可須 approved 且在效期內（安全網，應用層先檢查）
CREATE FUNCTION wishlists_relief_publish() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.type = 'relief' AND NEW.status = 'active' THEN
    IF NOT EXISTS (SELECT 1 FROM organizations WHERE id = NEW.org_id AND verification_status = 'verified') THEN
      RAISE EXCEPTION '組織未通過審核' USING ERRCODE = 'check_violation';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM org_permits WHERE id = NEW.permit_id AND org_id = NEW.org_id
                   AND status = 'approved' AND (now() AT TIME ZONE 'Asia/Taipei')::date BETWEEN valid_from AND valid_to) THEN
      RAISE EXCEPTION '許可未核准或已過期' USING ERRCODE = 'check_violation';
    END IF;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER wishlists_relief_publish BEFORE INSERT OR UPDATE OF status, org_id, permit_id ON wishlists
  FOR EACH ROW EXECUTE FUNCTION wishlists_relief_publish();

-- ---------- 收據 ----------
CREATE TABLE receipts (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  item_id     uuid        NOT NULL REFERENCES wishlist_items(id) ON DELETE CASCADE,
  wishlist_id uuid        NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
  qty         int         NOT NULL CHECK (qty > 0),
  note        text        CHECK (char_length(note) <= 200),
  recorded_by uuid        NOT NULL REFERENCES users(id),
  created_at  timestamptz NOT NULL DEFAULT now()
);

-- 個人清單看 owner；組織清單看 owner / coordinator 成員（建立者離職後不再有權限）
CREATE FUNCTION can_manage_wishlist(p_owner uuid, p_org uuid, p_uid uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
  SELECT CASE WHEN p_org IS NULL THEN p_owner = p_uid
    ELSE EXISTS (SELECT 1 FROM org_members WHERE org_id = p_org AND user_id = p_uid
                 AND role IN ('owner', 'coordinator') AND disabled_at IS NULL) END
$$;
