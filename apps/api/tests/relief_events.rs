//! 災害事件：建立、查重、編輯、合併（含併發）與下架。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn body(county: &str, date: &str) -> Value {
    json!({"name":"0403 地震","hazard_type":"earthquake","counties":[county],"occurred_on":date})
}
async fn post_ev(pool: &PgPool, tok: &str, org: Uuid, b: Value) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", &format!("/orgs/{org}/events"), &ck(tok), Some(b)).await;
    (s, v)
}
async fn merge(pool: &PgPool, tok: &str, from: Uuid, into: Uuid) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", &format!("/admin/relief-events/{from}/merge"), &ck(tok), Some(json!({"into": into}))).await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unverified_org_cannot_create_event(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "pending").await;
    member(&pool, o, u.id, "owner").await;
    let (s, v) = post_ev(&pool, &u.token, o, body("花蓮縣", "2026-10-20")).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::FORBIDDEN, "ORG_NOT_VERIFIED"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_then_similar_within_7_days_same_county(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    member(&pool, o, u.id, "coordinator").await;
    let (s, v) = post_ev(&pool, &u.token, o, body("花蓮縣", "2026-10-20")).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["merged_into"], Value::Null);
    let q = |c: &'static str, d: &'static str| {
        let pool = pool.clone();
        let t = u.token.clone();
        async move {
            let (_, _, v) = call(&pool, "GET", &format!("/relief-events/similar?county={c}&date={d}"), &ck(&t), None).await;
            v.as_array().unwrap().len()
        }
    };
    assert_eq!(q("花蓮縣", "2026-10-25").await, 1);
    assert_eq!(q("花蓮縣", "2026-11-05").await, 0);
    assert_eq!(q("臺東縣", "2026-10-25").await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invalid_county_422(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    member(&pool, o, u.id, "owner").await;
    let (s, v) = post_ev(&pool, &u.token, o, body("花蓮", "2026-10-20")).await;
    assert_eq!((s, v["errors"][0]["pointer"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "/counties"));
    let (s, _, _) = call(&pool, "GET", "/relief-events/similar?county=x&date=2026-10-20", &ck(&u.token), None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn other_org_can_attach_site_but_not_edit_event_and_lists_it(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let oa = org(&pool, a.id, "verified").await;
    let ob = org(&pool, b.id, "verified").await;
    member(&pool, oa, a.id, "owner").await;
    member(&pool, ob, b.id, "owner").await;
    let (e, _) = event(&pool, oa).await;
    let (s, _, v) = call(&pool, "PATCH", &format!("/relief-events/{e}"), &ck(&b.token), Some(json!({"name":"x"}))).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::FORBIDDEN, "ORG_ROLE_REQUIRED"));
    let (s, _, v) = call(&pool, "PATCH", &format!("/relief-events/{e}"), &ck(&a.token), Some(json!({"name":"新名稱"}))).await;
    assert_eq!((s, v["name"].as_str().unwrap()), (StatusCode::OK, "新名稱"));
    let list = |t: String, o: Uuid| {
        let pool = pool.clone();
        async move { call(&pool, "GET", &format!("/orgs/{o}/events"), &ck(&t), None).await.2.as_array().unwrap().len() }
    };
    assert_eq!(list(b.token.clone(), ob).await, 0);
    let p = permit(&pool, ob, "approved", 30).await;
    site(&pool, ob, b.id, e, p, "active").await;
    assert_eq!(list(b.token.clone(), ob).await, 1);
    assert_eq!(list(a.token.clone(), oa).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn merge_moves_sites_and_marks_merged(pool: PgPool) {
    let st = staff(&pool, "S").await;
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e1, _) = event(&pool, o).await;
    let (e2, _) = event(&pool, o).await;
    let w = site(&pool, o, u.id, e1, p, "active").await;
    let (s, _) = merge(&pool, &st.token, e1, e2).await;
    assert_eq!(s, StatusCode::OK);
    let ev: Uuid = sqlx::query_scalar("SELECT event_id FROM wishlists WHERE id=$1").bind(w.id).fetch_one(&pool).await.unwrap();
    assert_eq!(ev, e2);
    let m: Option<Uuid> = sqlx::query_scalar("SELECT merged_into FROM relief_events WHERE id=$1").bind(e1).fetch_one(&pool).await.unwrap();
    assert_eq!(m, Some(e2));
    // 已合併者不可再被合併，也不可當目標
    assert_eq!(merge(&pool, &st.token, e1, e2).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(merge(&pool, &st.token, e2, e1).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn merge_into_self_422(pool: PgPool) {
    let st = staff(&pool, "S").await;
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    let (e, _) = event(&pool, o).await;
    assert_eq!(merge(&pool, &st.token, e, e).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_opposite_merges_no_deadlock(pool: PgPool) {
    let st = staff(&pool, "S").await;
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    let (a, _) = event(&pool, o).await;
    let (b, _) = event(&pool, o).await;
    let (r1, r2) = tokio::join!(merge(&pool, &st.token, a, b), merge(&pool, &st.token, b, a));
    let mut codes = [r1.0.as_u16(), r2.0.as_u16()];
    codes.sort();
    assert_eq!(codes, [200, 422], "opposite merges deadlocked or both succeeded: {codes:?}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn hide_event_pauses_sites(pool: PgPool) {
    let st = staff(&pool, "S").await;
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e, _) = event(&pool, o).await;
    let w = site(&pool, o, u.id, e, p, "active").await;
    let path = format!("/admin/relief-events/{e}");
    let (s, _, _) = call(&pool, "PATCH", &path, &ck(&st.token), Some(json!({"moderation_status":"hidden"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, _) = call(&pool, "PATCH", &path, &ck(&st.token), Some(json!({"moderation_status":"hidden","moderation_reason":"不實"}))).await;
    assert_eq!(s, StatusCode::OK);
    let ss: String = sqlx::query_scalar("SELECT site_status FROM wishlists WHERE id=$1").bind(w.id).fetch_one(&pool).await.unwrap();
    assert_eq!(ss, "paused");
    let (s, _, _) = call(&pool, "PATCH", &path, &ck(&u.token), Some(json!({"moderation_status":"ok"}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
