//! 依興趣推薦精選商品：GET /users/{handle}/gift-picks
mod common;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::{befriend, call, ck, user};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

async fn prod(
    pool: &PgPool,
    r: &str,
    title: &str,
    ints: &[&str],
    recips: &[&str],
    occ: &[&str],
    sort: i32,
) {
    sqlx::query("INSERT INTO catalog_products (ref, title, price_amount, product_url, recipients, occasions, interests, sort_order, active) VALUES ($1,$2,100,'https://x.test',$3::text[],$4::text[],$5::text[],$6,true)")
        .bind(r).bind(title).bind(recips).bind(occ).bind(ints).bind(sort)
        .execute(pool).await.unwrap();
}
async fn set(pool: &PgPool, id: Uuid, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE users SET {sql} WHERE id = $1"
    )))
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}
async fn picks(pool: &PgPool, tok: Option<&str>, q: &str) -> (StatusCode, Value) {
    let h = tok.map(ck).unwrap_or_default();
    let (s, _, v) = call(
        pool,
        "GET",
        &format!("/users/alice/gift-picks{q}"),
        &h,
        None,
    )
    .await;
    (s, v)
}
fn refs(v: &Value) -> Vec<&str> {
    v["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["ref"].as_str().unwrap())
        .collect()
}
async fn alice(pool: &PgPool) -> common::TestUser {
    let a = user(pool, "Alice").await;
    set(pool, a.id, "handle='alice'").await;
    a
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn ranks_by_interest_overlap(pool: PgPool) {
    let a = alice(&pool).await;
    set(
        &pool,
        a.id,
        "interests='{coffee,tea}', interests_vis='public'",
    )
    .await;
    prod(&pool, "c", "C", &[], &["friend"], &[], 1).await;
    prod(&pool, "b", "B", &["coffee"], &[], &[], 2).await;
    prod(&pool, "a", "A", &["coffee", "tea"], &[], &[], 3).await;
    let (s, v) = picks(&pool, None, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["basis"], "interests");
    assert_eq!(refs(&v), ["a", "b"]);
    assert_eq!(
        v["products"][0]["matched"],
        serde_json::json!(["coffee", "tea"])
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn close_interests_hidden_from_friend(pool: PgPool) {
    let a = alice(&pool).await;
    let f = user(&pool, "F").await;
    befriend(&pool, a.id, f.id).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='close'").await;
    prod(&pool, "a", "A", &["coffee"], &["friend"], &[], 1).await;
    let (_, v) = picks(&pool, Some(&f.token), "").await;
    assert_eq!(v["basis"], "fallback");
    assert_eq!(v["products"][0]["matched"], serde_json::json!([]));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn anonymous_sees_public_only(pool: PgPool) {
    let a = alice(&pool).await;
    prod(&pool, "a", "A", &["coffee"], &["friend"], &[], 1).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='public'").await;
    assert_eq!(picks(&pool, None, "").await.1["basis"], "interests");
    set(&pool, a.id, "interests_vis='friends'").await;
    assert_eq!(picks(&pool, None, "").await.1["basis"], "fallback");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn dislikes_exclude_by_title_and_brand(pool: PgPool) {
    let a = alice(&pool).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='public', gift_dislikes='{香氛蠟燭}', gift_prefs_vis='public'").await;
    prod(&pool, "t", "手工香氛蠟燭", &["coffee"], &[], &[], 1).await;
    prod(&pool, "b", "禮盒", &["coffee"], &[], &[], 2).await;
    prod(&pool, "ok", "咖啡杯", &["coffee"], &[], &[], 3).await;
    let brand: Uuid = sqlx::query_scalar(
        "INSERT INTO brands (slug, name) VALUES ('bx','香氛蠟燭工坊') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE catalog_products SET brand_id=$1 WHERE ref='b'")
        .bind(brand)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(refs(&picks(&pool, None, "").await.1), ["ok"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn hidden_dislikes_do_not_filter(pool: PgPool) {
    let a = alice(&pool).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='public', gift_dislikes='{香氛蠟燭}', gift_prefs_vis='private'").await;
    prod(&pool, "t", "手工香氛蠟燭", &["coffee"], &[], &[], 1).await;
    assert_eq!(refs(&picks(&pool, None, "").await.1), ["t"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn single_char_dislike_ignored(pool: PgPool) {
    let a = alice(&pool).await;
    set(
        &pool,
        a.id,
        "interests='{tea}', interests_vis='public', gift_dislikes='{茶}', gift_prefs_vis='public'",
    )
    .await;
    prod(&pool, "t", "茶具組", &["tea"], &[], &[], 1).await;
    assert_eq!(refs(&picks(&pool, None, "").await.1), ["t"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn fallback_when_catalog_untagged(pool: PgPool) {
    let a = alice(&pool).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='public'").await;
    prod(&pool, "x", "X", &[], &["partner"], &[], 1).await;
    prod(&pool, "f2", "F2", &[], &["friend"], &[], 2).await;
    prod(&pool, "f1", "F1", &[], &["friend"], &[], 1).await;
    let (_, v) = picks(&pool, None, "").await;
    assert_eq!(v["basis"], "fallback");
    assert_eq!(refs(&v), ["f1", "f2"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn fallback_prefers_birthday_within_30_days(pool: PgPool) {
    let a = alice(&pool).await;
    let d = (Utc::now() + Duration::hours(8) + Duration::days(10))
        .format("%m-%d")
        .to_string();
    set(
        &pool,
        a.id,
        &format!("birthday='{d}', birthday_visibility='public'"),
    )
    .await;
    prod(&pool, "f", "F", &[], &["friend"], &[], 1).await;
    prod(&pool, "b", "B", &[], &[], &["birthday"], 2).await;
    assert_eq!(refs(&picks(&pool, None, "").await.1), ["b"]);
    // 生日不可見 → 回到 friend 推薦
    set(&pool, a.id, "birthday_visibility='private'").await;
    assert_eq!(refs(&picks(&pool, None, "").await.1), ["f"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn occasion_filters_candidates(pool: PgPool) {
    let a = alice(&pool).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='public'").await;
    prod(&pool, "h", "H", &["coffee"], &[], &["housewarming"], 1).await;
    prod(&pool, "b", "B", &["coffee"], &[], &["birthday"], 2).await;
    assert_eq!(
        refs(&picks(&pool, None, "?occasion=housewarming").await.1),
        ["h"]
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn blocked_cases(pool: PgPool) {
    let a = alice(&pool).await;
    let v = user(&pool, "V").await;
    prod(&pool, "f", "F", &[], &["friend"], &[], 1).await;
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1,$2)")
        .bind(a.id)
        .bind(v.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        picks(&pool, Some(&v.token), "").await.0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("DELETE FROM user_blocks")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1,$2)")
        .bind(v.id)
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = picks(&pool, Some(&v.token), "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b,
        serde_json::json!({"basis":"fallback","products":[],"excluded":[]})
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn no_store_and_validation(pool: PgPool) {
    alice(&pool).await;
    let (_, h, _) = call(&pool, "GET", "/users/alice/gift-picks", &[], None).await;
    assert_eq!(h.get("cache-control").unwrap(), "no-store");
    assert_eq!(
        picks(&pool, None, "?occasion=x").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        picks(&pool, None, "?limit=51").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        picks(&pool, None, "?limit=0").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn excluded_lists_disliked_refs(pool: PgPool) {
    let a = alice(&pool).await;
    set(&pool, a.id, "interests='{coffee}', interests_vis='public', gift_dislikes='{香氛蠟燭}', gift_prefs_vis='public'").await;
    prod(
        &pool,
        "t",
        "手工香氛蠟燭",
        &[],
        &["friend"],
        &["birthday"],
        1,
    )
    .await;
    prod(&pool, "x", "香氛蠟燭 其他場合", &[], &[], &["wedding"], 2).await;
    prod(&pool, "ok", "咖啡杯", &[], &["friend"], &["birthday"], 3).await;
    let (_, v) = picks(&pool, None, "?occasion=birthday").await;
    assert_eq!(v["excluded"], serde_json::json!(["t"]));
    set(&pool, a.id, "gift_prefs_vis='private'").await;
    let (_, v) = picks(&pool, None, "?occasion=birthday").await;
    assert_eq!(v["excluded"], serde_json::json!([]));
}
