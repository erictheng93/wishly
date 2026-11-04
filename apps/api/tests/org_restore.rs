//! 撤銷後 7 天內恢復組織認證。
mod common;
use axum::http::StatusCode;
use common::{call, call_app, ck, staff, user, TestUser};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;
use wishly_api::{app, AppState};

async fn revoke(pool: &PgPool, st: &str, o: Uuid) {
    let (s, _, b) = call(
        pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(st),
        Some(json!({"decision":"reject","note":"撤銷"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}
async fn restore(pool: &PgPool, st: &str, o: Uuid) -> (StatusCode, serde_json::Value) {
    let (s, _, b) = call(
        pool,
        "POST",
        &format!("/admin/orgs/{o}/restore"),
        &ck(st),
        Some(json!({"reason":"誤判"})),
    )
    .await;
    (s, b)
}
async fn site_state(pool: &PgPool, id: Uuid) -> (String, bool) {
    sqlx::query_as("SELECT status::text, revoke_paused_at IS NOT NULL FROM wishlists WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn restore_within_window(pool: PgPool) {
    let cdn = common::cdn::init();
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    common::member(&pool, o, owner, "owner").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (ev, evs) = common::event(&pool, o).await;
    let live = common::site(&pool, o, owner, ev, p, "active").await;
    let was_draft = common::site(&pool, o, owner, ev, p, "draft").await;
    revoke(&pool, &st, o).await;
    assert_eq!(site_state(&pool, live.id).await, ("draft".into(), true));
    assert_eq!(
        site_state(&pool, was_draft.id).await,
        ("draft".into(), false)
    );
    // 詳情帶出可恢復資訊
    let (_, _, d) = call(&pool, "GET", &format!("/admin/orgs/{o}"), &ck(&st), None).await;
    assert!(
        d["org"]["restorable_until"].is_string() && d["org"]["revoked_at"].is_string(),
        "{d}"
    );
    assert_eq!(d["org"]["revoke_paused_sites"], 1);
    let (_, _, l) = call(&pool, "GET", "/admin/orgs?status=rejected", &ck(&st), None).await;
    assert_eq!(l["data"][0]["revoke_paused_sites"], 1, "{l}");

    let (s, b) = restore(&pool, &st, o).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["org"]["verification_status"], "verified");
    assert_eq!(b["restored"], json!([live.slug.trim()]));
    assert_eq!(b["skipped"], json!([]));
    assert_eq!(site_state(&pool, live.id).await, ("active".into(), false));
    assert_eq!(
        site_state(&pool, was_draft.id).await,
        ("draft".into(), false)
    );
    let (v, r, ra): (String, bool, bool) = sqlx::query_as("SELECT verification_status, revoked_at IS NULL, resubmit_after IS NULL FROM organizations WHERE id=$1").bind(o).fetch_one(&pool).await.unwrap();
    assert_eq!((v.as_str(), r, ra), ("verified", true, true));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='org.review' AND payload->>'decision'='verified' AND user_id=$1").bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='org.restore'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    cdn.wait_for(&live.slug).await;
    cdn.wait_for_url(&common::cdn::event_url(&evs)).await;
    // 恢復後不再可恢復
    let (s, b) = restore(&pool, &st, o).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("RESTORE_NOT_ALLOWED")),
        "{b}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn restore_skips_sites_with_invalid_permit(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    let p1 = common::permit(&pool, o, "approved", 30).await;
    let p2 = common::permit(&pool, o, "approved", 30).await;
    let (ev, _) = common::event(&pool, o).await;
    let ok = common::site(&pool, o, owner, ev, p1, "active").await;
    let bad = common::site(&pool, o, owner, ev, p2, "active").await;
    revoke(&pool, &st, o).await;
    sqlx::query("UPDATE org_permits SET status='rejected' WHERE id=$1")
        .bind(p2)
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = restore(&pool, &st, o).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["restored"], json!([ok.slug.trim()]));
    assert_eq!(
        b["skipped"],
        json!([{"slug": bad.slug.trim(), "reason": "permit_invalid"}])
    );
    assert_eq!(site_state(&pool, bad.id).await, ("draft".into(), false));
    assert_eq!(site_state(&pool, ok.id).await, ("active".into(), false));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn restore_rejected_cases(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let TestUser { token: plain, .. } = user(&pool, "plain").await;
    let o = common::org(&pool, owner, "verified").await;
    common::permit(&pool, o, "approved", 30).await;
    revoke(&pool, &st, o).await;
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/restore"),
        &ck(&plain),
        Some(json!({"reason":"x"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/restore"),
        &ck(&st),
        Some(json!({"reason":"  "})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    // 超過 7 天
    sqlx::query("UPDATE organizations SET revoked_at = now() - interval '8 days' WHERE id=$1")
        .bind(o)
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = restore(&pool, &st, o).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("RESTORE_NOT_ALLOWED")),
        "{b}"
    );
    let (_, _, d) = call(&pool, "GET", &format!("/admin/orgs/{o}"), &ck(&st), None).await;
    assert!(d["org"]["restorable_until"].is_null());
    // 待審被退件（非撤銷）
    let o2 = common::org(&pool, owner, "pending").await;
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o2}/review"),
        &ck(&st),
        Some(json!({"decision":"reject","note":"x"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = restore(&pool, &st, o2).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("RESTORE_NOT_ALLOWED")),
        "{b}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn approve_clears_revoked_at(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (ev, _) = common::event(&pool, o).await;
    let s1 = common::site(&pool, o, owner, ev, p, "active").await;
    revoke(&pool, &st, o).await;
    // 模擬重新送審後回到 pending 再核准
    sqlx::query("UPDATE organizations SET verification_status='pending' WHERE id=$1")
        .bind(o)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st),
        Some(json!({"decision":"approve"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let r: bool = sqlx::query_scalar("SELECT revoked_at IS NULL FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(r);
    assert_eq!(site_state(&pool, s1.id).await, ("draft".into(), false));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_restores_apply_once(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: s1, .. } = staff(&pool, "s1").await;
    let TestUser { token: s2, .. } = staff(&pool, "s2").await;
    let o = common::org(&pool, owner, "verified").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (ev, _) = common::event(&pool, o).await;
    common::site(&pool, o, owner, ev, p, "active").await;
    revoke(&pool, &s1, o).await;
    let app = app(AppState { pool: pool.clone() });
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(o)
        .execute(&mut *gate)
        .await
        .unwrap();
    let uri = format!("/admin/orgs/{o}/restore");
    let go = |tok: String| {
        let (app, uri) = (app.clone(), uri.clone());
        tokio::spawn(async move {
            call_app(
                &app,
                "POST",
                &uri,
                &ck(&tok),
                Some(json!({"reason":"誤判"})),
            )
            .await
        })
    };
    let (a, b) = (go(s1), go(s2));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock'
                   AND query LIKE '%FROM organizations WHERE id = %FOR UPDATE' AND query LIKE '%revoked_at%'")
                .fetch_one(&pool).await.unwrap();
            if n >= 2 { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("expected 2 restore statements blocked on the org row lock");
    gate.commit().await.unwrap();
    let mut codes = [a.await.unwrap().0, b.await.unwrap().0];
    codes.sort();
    assert_eq!(codes, [StatusCode::OK, StatusCode::CONFLICT]);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='org.restore'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}
