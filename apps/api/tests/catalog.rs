//! 品牌館與精選商品目錄：公開查詢（過濾 / 分頁 / 不讀 cookie）與後台維護
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn brand(pool: &PgPool, slug: &str, active: bool) -> Uuid {
    sqlx::query_scalar("INSERT INTO brands (slug, name, active) VALUES ($1,$1,$2) RETURNING id")
        .bind(slug)
        .bind(active)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn product(pool: &PgPool, r: &str, brand_id: Option<Uuid>, price: i64, active: bool) {
    sqlx::query("INSERT INTO catalog_products (ref, brand_id, title, price_amount, product_url, recipients, occasions, active) VALUES ($1,$2,$1,$3,'https://x.test','{partner}','{birthday}',$4)")
        .bind(r).bind(brand_id).bind(price).bind(active).execute(pool).await.unwrap();
}

async fn refs(pool: &PgPool, q: &str) -> Vec<String> {
    let (s, h, v) = call(
        pool,
        "GET",
        &format!("/public/catalog/products{q}"),
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h["cache-control"], "public, max-age=60");
    let mut r: Vec<String> = v["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["ref"].as_str().unwrap().into())
        .collect();
    r.sort();
    r
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_list_hides_inactive(pool: PgPool) {
    let on = brand(&pool, "on-brand", true).await;
    let off = brand(&pool, "off-brand", false).await;
    product(&pool, "a", None, 100, true).await;
    product(&pool, "b", Some(on), 100, true).await;
    product(&pool, "c", Some(off), 100, true).await; // 品牌停用
    product(&pool, "d", None, 100, false).await; // 商品停用
    assert_eq!(refs(&pool, "").await, ["a", "b"]);
    assert_eq!(refs(&pool, "?brand=on-brand").await, ["b"]);
    assert!(refs(&pool, "?brand=off-brand").await.is_empty());
    // 回傳格式
    let (_, _, v) = call(
        &pool,
        "GET",
        "/public/catalog/products?brand=on-brand",
        &[],
        None,
    )
    .await;
    let p = &v["products"][0];
    assert_eq!(p["brand"], json!({"slug":"on-brand","name":"on-brand"}));
    assert_eq!(p["image_url"], Value::Null);
    assert_eq!(p["recipients"], json!(["partner"]));
    assert_eq!(v["next_page"], Value::Null);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn filters_and_budget_boundaries(pool: PgPool) {
    for (r, price) in [
        ("p999", 999),
        ("p1000", 1000),
        ("p3000", 3000),
        ("p3001", 3001),
    ] {
        product(&pool, r, None, price, true).await;
    }
    sqlx::query("UPDATE catalog_products SET recipients='{baby,elder}', occasions='{full_month}' WHERE ref='p999'").execute(&pool).await.unwrap();
    assert_eq!(refs(&pool, "?budget=lt1000").await, ["p999"]);
    assert_eq!(refs(&pool, "?budget=1000-3000").await, ["p1000", "p3000"]);
    assert_eq!(refs(&pool, "?budget=gt3000").await, ["p3001"]);
    assert_eq!(refs(&pool, "?recipient=baby").await, ["p999"]);
    assert_eq!(
        refs(&pool, "?occasion=full_month&recipient=elder").await,
        ["p999"]
    );
    assert_eq!(
        refs(&pool, "?occasion=birthday").await,
        ["p1000", "p3000", "p3001"]
    );
    for q in ["?budget=x", "?recipient=伴侶", "?occasion=x"] {
        let (s, _, v) = call(
            &pool,
            "GET",
            &format!("/public/catalog/products{q}"),
            &[],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{q}");
        assert_eq!(v["code"], "VALIDATION_FAILED");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn paging(pool: PgPool) {
    for i in 0..25 {
        product(&pool, &format!("r{i:02}"), None, 100, true).await;
    }
    let (_, _, v) = call(&pool, "GET", "/public/catalog/products", &[], None).await;
    assert_eq!(v["products"].as_array().unwrap().len(), 24);
    assert_eq!(v["next_page"], 2);
    let (_, _, v) = call(&pool, "GET", "/public/catalog/products?page=2", &[], None).await;
    assert_eq!(v["products"].as_array().unwrap().len(), 1);
    assert_eq!(v["next_page"], Value::Null);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_brands_and_cookies_ignored(pool: PgPool) {
    brand(&pool, "on-brand", true).await;
    brand(&pool, "off-brand", false).await;
    let u = user(&pool, "路人").await;
    let (s, h, v) = call(&pool, "GET", "/public/brands", &ck(&u.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "public, max-age=60");
    assert_eq!(
        v["brands"],
        json!([{"slug":"on-brand","name":"on-brand","description":null,"logo_url":null}])
    );
    let (_, _, v) = call(&pool, "GET", "/public/brands/on-brand", &[], None).await;
    assert_eq!(v["brand"]["slug"], "on-brand");
    assert!(v["brand"].get("website_url").is_some());
    for slug in ["off-brand", "nope"] {
        let (s, _, _) = call(&pool, "GET", &format!("/public/brands/{slug}"), &[], None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{slug}");
    }
    // 有無 cookie 回應相同
    let a = call(&pool, "GET", "/public/catalog/products", &[], None).await;
    let b = call(
        &pool,
        "GET",
        "/public/catalog/products",
        &ck(&u.token),
        None,
    )
    .await;
    assert_eq!(a.2, b.2);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_requires_staff(pool: PgPool) {
    let u = user(&pool, "一般").await;
    for (m, p) in [("GET", "/admin/brands"), ("GET", "/admin/catalog/products")] {
        assert_eq!(
            call(&pool, m, p, &[], None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&pool, m, p, &ck(&u.token), None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let b = Some(json!({"slug":"abc","name":"x"}));
    assert_eq!(
        call(&pool, "POST", "/admin/brands", &ck(&u.token), b)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_brand_lifecycle(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let h = ck(&s.token);
    let (st, _, v) = call(&pool, "POST", "/admin/brands", &h, Some(json!({"slug":"hario","name":"HARIO","website_url":"https://hario.test","sort_order":3}))).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let id = v["brand"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["brand"]["active"], false);
    assert_eq!(v["brand"]["logo_status"], "none");
    // 預設停用 → 公開不可見
    assert_eq!(
        call(&pool, "GET", "/public/brands/hario", &[], None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // 重複 slug
    let (st, _, v) = call(
        &pool,
        "POST",
        "/admin/brands",
        &h,
        Some(json!({"slug":"hario","name":"x"})),
    )
    .await;
    assert_eq!(
        (st, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "SLUG_TAKEN")
    );
    // 壞輸入
    for b in [
        json!({"slug":"A","name":"x"}),
        json!({"slug":"ok-1"}),
        json!({"slug":"ok-2","name":"x","website_url":"ftp://a"}),
    ] {
        assert_eq!(
            call(&pool, "POST", "/admin/brands", &h, Some(b)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // patch：slug 不可改；logo_key 須為 catalog/ 前綴
    let uri = format!("/admin/brands/{id}");
    assert_eq!(
        call(&pool, "PATCH", &uri, &h, Some(json!({"slug":"new"})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let bad = format!("items/{}.png", Uuid::new_v4());
    assert_eq!(
        call(&pool, "PATCH", &uri, &h, Some(json!({"logo_key":bad})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let key = format!("catalog/{}.png", Uuid::new_v4());
    let (st, _, v) = call(
        &pool,
        "PATCH",
        &uri,
        &h,
        Some(json!({"active":true,"logo_key":key})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["brand"]["logo_status"], "ready");
    assert!(v["brand"]["logo_url"].as_str().unwrap().contains(&key));
    assert_eq!(
        call(&pool, "GET", "/public/brands/hario", &[], None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/admin/brands/{}", Uuid::new_v4()),
            &h,
            Some(json!({}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, _, v) = call(&pool, "GET", "/admin/brands", &h, None).await;
    assert_eq!(v["brands"].as_array().unwrap().len(), 1);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE entity='brands' AND actor_id=$1 AND action IN ('brand.create','brand.update')")
        .bind(s.id).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_product_lifecycle(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let h = ck(&s.token);
    let bid = brand(&pool, "b1", true).await;
    let ok = json!({"ref":"cup-1","title":"杯子","price_amount":1200,"product_url":"https://shop.test/cup","brand_id":bid,
        "recipients":["friend","friend","baby"],"occasions":["birthday"],"active":true});
    let (st, _, v) = call(
        &pool,
        "POST",
        "/admin/catalog/products",
        &h,
        Some(ok.clone()),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let p = &v["product"];
    assert_eq!(p["recipients"], json!(["friend", "baby"]), "去重");
    assert_eq!(p["image_status"], "none");
    let id = p["id"].as_str().unwrap().to_string();
    assert_eq!(refs(&pool, "").await, ["cup-1"]);
    // 重複 ref
    let (st, _, v) = call(
        &pool,
        "POST",
        "/admin/catalog/products",
        &h,
        Some(ok.clone()),
    )
    .await;
    assert_eq!(
        (st, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "REF_TAKEN")
    );
    // 驗證失敗
    for patch in [
        json!({"ref":"other"}),
        json!({"recipients":["伴侶"]}),
        json!({"occasions":["x"]}),
        json!({"product_url":"javascript:alert(1)"}),
        json!({"title":"x".repeat(121)}),
        json!({"spec":"x".repeat(201)}),
        json!({"note":"x".repeat(301)}),
        json!({"price_amount":-1}),
        json!({"brand_id":Uuid::new_v4()}),
    ] {
        let (st, _, v) = call(
            &pool,
            "PATCH",
            &format!("/admin/catalog/products/{id}"),
            &h,
            Some(patch.clone()),
        )
        .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{patch} {v}");
    }
    for miss in ["ref", "title", "price_amount", "product_url"] {
        let mut b = ok.clone();
        b["ref"] = json!(format!("m-{miss}"));
        if miss != "ref" {
            b.as_object_mut().unwrap().remove(miss);
        } else {
            b["ref"] = json!("bad ref!");
        }
        let (st, _, _) = call(&pool, "POST", "/admin/catalog/products", &h, Some(b)).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{miss}");
    }
    // patch：停用、品牌改 null、清空 spec
    let (st, _, v) = call(
        &pool,
        "PATCH",
        &format!("/admin/catalog/products/{id}"),
        &h,
        Some(json!({"active":false,"brand_id":null,"price_amount":0})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["product"]["ref"], "cup-1");
    assert_eq!(v["product"]["brand_id"], Value::Null);
    assert!(refs(&pool, "").await.is_empty());
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/admin/catalog/products?brand_id={bid}"),
        &h,
        None,
    )
    .await;
    assert!(v["products"].as_array().unwrap().is_empty());
    let (_, _, v) = call(&pool, "GET", "/admin/catalog/products", &h, None).await;
    assert_eq!(v["products"].as_array().unwrap().len(), 1, "含停用");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity='catalog_products' AND actor_id=$1",
    )
    .bind(s.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 2);
}
