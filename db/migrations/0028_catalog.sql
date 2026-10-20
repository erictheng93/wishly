-- 品牌館與精選商品目錄（營運人員在後台維護；公開 API 見 catalog.rs）
-- brands 是展示用的「品牌」，不是 docs/04 的 merchants（供貨/下單對象）；兩者不要混用。
CREATE TABLE brands (
  id           uuid PRIMARY KEY DEFAULT uuid_v7(),
  slug         text        NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9-]{2,40}$'),
  name         text        NOT NULL,
  description  text,
  logo_key     text,
  logo_status  image_status NOT NULL DEFAULT 'none',
  website_url  text,
  active       boolean     NOT NULL DEFAULT false,
  sort_order   int         NOT NULL DEFAULT 0,
  created_at   timestamptz NOT NULL DEFAULT now(),
  updated_at   timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT brands_logo_status_key CHECK ((logo_status <> 'none' OR logo_key IS NULL) AND (logo_status NOT IN ('pending', 'ready') OR logo_key IS NOT NULL))
);
CREATE TRIGGER brands_set_updated_at BEFORE UPDATE ON brands FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 與 docs/04 的 catalog_products 是同一張表：P2-A 再補 merchant_id / merchant_sku / shipping_fee_amount / stock_status
-- （屆時既有列回填後才加 NOT NULL）。ref 即 wishlist_items.catalog_ref，建立後不可改。
CREATE TABLE catalog_products (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  ref           text        NOT NULL UNIQUE CHECK (ref ~ '^[A-Za-z0-9_-]{1,64}$'),
  brand_id      uuid        REFERENCES brands(id) ON DELETE RESTRICT,   -- NULL = Wishly 精選
  title         text        NOT NULL,
  spec          text,
  note          text,
  price_amount  bigint      NOT NULL CHECK (price_amount >= 0),          -- 新台幣元
  image_key     text,
  image_status  image_status NOT NULL DEFAULT 'none',
  product_url   text        NOT NULL,
  recipients    text[]      NOT NULL DEFAULT '{}',                       -- partner | elder | baby | friend（應用層驗證）
  occasions     text[]      NOT NULL DEFAULT '{}',                       -- birthday | full_month | housewarming | holiday
  active        boolean     NOT NULL DEFAULT false,
  sort_order    int         NOT NULL DEFAULT 0,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT catalog_products_image_status_key CHECK ((image_status <> 'none' OR image_key IS NULL) AND (image_status NOT IN ('pending', 'ready') OR image_key IS NOT NULL))
);
CREATE TRIGGER catalog_products_set_updated_at BEFORE UPDATE ON catalog_products FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE INDEX catalog_products_list_idx ON catalog_products (sort_order, created_at DESC) WHERE active;
CREATE INDEX catalog_products_brand_idx ON catalog_products (brand_id);
