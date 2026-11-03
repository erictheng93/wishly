//! 組織稽核軌跡：範圍、角色、捐助排除與演員遮蔽。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn log(
    pool: &PgPool,
    actor: &str,
    actor_id: Option<Uuid>,
    action: &str,
    entity: &str,
    eid: Option<Uuid>,
    diff: Value,
) {
    sqlx::query("INSERT INTO audit_logs(actor_type,actor_id,action,entity,entity_id,diff) VALUES($1::actor_type,$2,$3,$4,$5,$6)")
        .bind(actor).bind(actor_id).bind(action).bind(entity).bind(eid).bind(diff)
        .execute(pool).await.unwrap();
}
async fn get_logs(pool: &PgPool, org: Uuid, tok: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "GET",
        &format!("/orgs/{org}/audit-logs"),
        &ck(tok),
        None,
    )
    .await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn scope_roles_and_masking(pool: PgPool) {
    let a = user(&pool, "A").await;
    let vol = user(&pool, "V").await;
    let out = user(&pool, "O").await;
    let st = staff(&pool, "S").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    member(&pool, o, vol.id, "volunteer").await;
    let other = org(&pool, out.id, "verified").await;
    let (ev, _) = event(&pool, o).await;
    let pm = permit(&pool, o, "approved", 30).await;
    let s = site(&pool, o, a.id, ev, pm, "active").await;
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'水',5,'water','high') RETURNING id")
        .bind(s.id).fetch_one(&pool).await.unwrap();

    log(
        &pool,
        "user",
        Some(a.id),
        "org.update",
        "organizations",
        Some(o),
        json!({"org_id":o}),
    )
    .await;
    log(
        &pool,
        "user",
        Some(a.id),
        "item.update",
        "wishlist_items",
        Some(it),
        json!({}),
    )
    .await;
    log(
        &pool,
        "staff",
        Some(st.id),
        "org.review",
        "organizations",
        Some(o),
        json!({}),
    )
    .await;
    log(
        &pool,
        "user",
        Some(a.id),
        "claim.create",
        "claims",
        Some(it),
        json!({"org_id":o}),
    )
    .await;
    log(
        &pool,
        "user",
        Some(a.id),
        "contribution.create",
        "contributions",
        Some(s.id),
        json!({}),
    )
    .await;
    log(
        &pool,
        "user",
        Some(out.id),
        "org.update",
        "organizations",
        Some(other),
        json!({"org_id":other}),
    )
    .await;

    let (c, _) = get_logs(&pool, o, &vol.token).await;
    assert_eq!(c, StatusCode::FORBIDDEN);
    let (c, _) = get_logs(&pool, o, &out.token).await;
    assert_eq!(c, StatusCode::NOT_FOUND);

    let (c, v) = get_logs(&pool, o, &a.token).await;
    assert_eq!(c, StatusCode::OK, "{v}");
    let d = v["data"].as_array().unwrap();
    let acts: Vec<&str> = d.iter().map(|r| r["action"].as_str().unwrap()).collect();
    assert_eq!(d.len(), 3, "{acts:?}");
    assert!(acts.contains(&"item.update") && acts.contains(&"org.review"));
    let staff_row = d.iter().find(|r| r["action"] == "org.review").unwrap();
    assert_eq!(staff_row["actor"]["type"], "staff");
    assert!(staff_row["actor"]["name"].is_null());
    let user_row = d.iter().find(|r| r["action"] == "org.update").unwrap();
    assert_eq!(user_row["actor"]["name"], "A");
    let raw = v.to_string();
    assert!(!raw.contains("actor_id") && !raw.contains(&st.id.to_string()));
}
