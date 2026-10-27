//! 據點實收登記：權限、結束據點、跨組織、冪等與合計。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

struct Fx {
    org: Uuid,
    vol: TestUser,
    site: TestList,
    item: Uuid,
}
async fn fx(pool: &PgPool, status: &str) -> Fx {
    let owner = user(pool, "O").await;
    let vol = user(pool, "志工").await;
    let o = org(pool, owner.id, "verified").await;
    member(pool, o, owner.id, "owner").await;
    member(pool, o, vol.id, "volunteer").await;
    let p = permit(pool, o, "approved", 30).await;
    let (e, _) = event(pool, o).await;
    let site = site(pool, o, owner.id, e, p, "active").await;
    sqlx::query("UPDATE wishlists SET site_status=$2 WHERE id=$1").bind(site.id).bind(status).execute(pool).await.unwrap();
    let item = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'礦泉水',100,'water','high') RETURNING id")
        .bind(site.id).fetch_one(pool).await.unwrap();
    Fx { org: o, vol, site, item }
}
async fn rec(pool: &PgPool, f: &Fx, qty: i64, key: Uuid) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut h = ck(&f.vol.token);
    h.extend(idem(key));
    call(pool, "POST", &format!("/orgs/{}/receipts", f.org), &h, Some(json!({"item_id": f.item, "qty": qty, "note": "一箱"}))).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn volunteer_records_receipt(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let (s, _, v) = rec(&pool, &f, 5, Uuid::new_v4()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["qty"], 5);
    rec(&pool, &f, 7, Uuid::new_v4()).await;
    let (s, _, v) = call(&pool, "GET", &format!("/orgs/{}/sites/{}/receipts", f.org, f.site.id), &ck(&f.vol.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v[0]["item_id"], json!(f.item));
    assert_eq!(v[0]["total"], 12);
    assert_eq!(v[0]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(v[0]["entries"][0]["qty"], 7); // 新的在前
    assert_eq!(v[0]["entries"][0]["recorded_by_name"], "志工");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn closed_site_rejects_but_paused_allows(pool: PgPool) {
    let f = fx(&pool, "closed").await;
    let (s, _, v) = rec(&pool, &f, 1, Uuid::new_v4()).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "SITE_CLOSED"));
    sqlx::query("UPDATE wishlists SET site_status='paused' WHERE id=$1").bind(f.site.id).execute(&pool).await.unwrap();
    assert_eq!(rec(&pool, &f, 1, Uuid::new_v4()).await.0, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn item_of_other_org_404(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let g = fx(&pool, "active").await;
    let mut h = ck(&f.vol.token);
    h.extend(idem(Uuid::new_v4()));
    let (s, _, _) = call(&pool, "POST", &format!("/orgs/{}/receipts", f.org), &h, Some(json!({"item_id": g.item, "qty": 1}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // 非成員
    let (s, _, _) = call(&pool, "GET", &format!("/orgs/{}/sites/{}/receipts", g.org, g.site.id), &ck(&f.vol.token), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotent_replay(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let k = Uuid::new_v4();
    let (_, h1, _) = rec(&pool, &f, 3, k).await;
    assert!(h1.get("idempotency-replayed").is_none());
    let (s, h2, _) = rec(&pool, &f, 3, k).await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(h2.get("idempotency-replayed").is_some());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM receipts").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}
