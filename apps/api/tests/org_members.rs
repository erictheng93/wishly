//! 組織成員與邀請：邀請接受流程、email 比對、最後一位 owner 保護與停用。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// 讓使用者擁有已驗證的 email 登入身分
async fn verify(pool: &PgPool, user: Uuid, email: &str) {
    sqlx::query("INSERT INTO auth_identities(user_id,provider,provider_uid,email) VALUES($1,'email',$2,$2)")
        .bind(user).bind(email).execute(pool).await.unwrap();
}
async fn invite(pool: &PgPool, org: Uuid, tok: &str, email: &str, role: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", &format!("/orgs/{org}/invites"), &ck(tok), Some(json!({"email":email,"role":role}))).await;
    (s, v)
}
fn token_of(v: &Value) -> String {
    v["url"].as_str().unwrap().rsplit('/').next().unwrap().to_string()
}
async fn accept(pool: &PgPool, tok: &str, invite_token: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", &format!("/org-invites/{invite_token}/accept"), &ck(tok), None).await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_accept_flow(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    verify(&pool, b.id, "b@x.com").await;
    sqlx::query("UPDATE users SET email='b@x.com' WHERE id=$1").bind(b.id).execute(&pool).await.unwrap();
    let (s, v) = invite(&pool, o, &a.token, "B@x.com", "coordinator").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert!(v["url"].as_str().unwrap().contains("/org/invite/"));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='org.invite'").bind(b.id).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let t = token_of(&v);
    let (s, _, p) = call(&pool, "GET", &format!("/org-invites/{t}"), &ck(&b.token), None).await;
    assert_eq!((s, p["email"].as_str()), (StatusCode::OK, Some("b@x.com")));
    let (s, r) = accept(&pool, &b.token, &t).await;
    assert_eq!((s, r["role"].as_str()), (StatusCode::OK, Some("coordinator")), "{r}");
    let (_, _, me) = call(&pool, "GET", "/me", &ck(&b.token), None).await;
    assert_eq!(me["orgs"][0]["role"], "coordinator");
    let (s, r) = accept(&pool, &b.token, &t).await;
    assert_eq!((s, r["code"].as_str()), (StatusCode::GONE, Some("INVITE_EXPIRED")));
    let (_, _, list) = call(&pool, "GET", &format!("/orgs/{o}/members"), &ck(&b.token), None).await;
    assert_eq!(list[0]["role"], "owner");
    assert_eq!(list[1]["display_name"], "B");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_email_mismatch(pool: PgPool) {
    let a = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    verify(&pool, c.id, "c@x.com").await;
    let (_, v) = invite(&pool, o, &a.token, "b@x.com", "volunteer").await;
    let (s, r) = accept(&pool, &c.token, &token_of(&v)).await;
    assert_eq!((s, r["code"].as_str(), r["email"].as_str()), (StatusCode::FORBIDDEN, Some("INVITE_EMAIL_MISMATCH"), Some("b@x.com")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_expired_or_cancelled(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    verify(&pool, b.id, "b@x.com").await;
    let (_, v1) = invite(&pool, o, &a.token, "b@x.com", "volunteer").await;
    let (_, v2) = invite(&pool, o, &a.token, "b@x.com", "volunteer").await;
    let (s, _, _) = call(&pool, "DELETE", &format!("/orgs/{o}/invites/{}", v1["id"].as_str().unwrap()), &ck(&a.token), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(accept(&pool, &b.token, &token_of(&v1)).await.0, StatusCode::GONE);
    sqlx::query("UPDATE org_invites SET expires_at=now()-interval '1 second' WHERE id=$1").bind(v2["id"].as_str().unwrap().parse::<Uuid>().unwrap()).execute(&pool).await.unwrap();
    assert_eq!(accept(&pool, &b.token, &token_of(&v2)).await.0, StatusCode::GONE);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn disabled_member_can_rejoin_via_invite(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    member(&pool, o, b.id, "volunteer").await;
    verify(&pool, b.id, "b@x.com").await;
    let uri = format!("/orgs/{o}/members/{}", b.id);
    let (s, _, _) = call(&pool, "PATCH", &uri, &ck(&a.token), Some(json!({"disabled":true}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = invite(&pool, o, &a.token, "b@x.com", "coordinator").await;
    assert_eq!(accept(&pool, &b.token, &token_of(&v)).await.0, StatusCode::OK);
    let (role, off): (String, bool) = sqlx::query_as("SELECT role::text, disabled_at IS NOT NULL FROM org_members WHERE org_id=$1 AND user_id=$2").bind(o).bind(b.id).fetch_one(&pool).await.unwrap();
    assert_eq!((role.as_str(), off), ("coordinator", false));
    let (_, v) = invite(&pool, o, &a.token, "b@x.com", "coordinator").await;
    assert_eq!(accept(&pool, &b.token, &token_of(&v)).await.1["code"], "ALREADY_MEMBER");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn last_owner_cannot_demote_or_disable_self(pool: PgPool) {
    let a = user(&pool, "A").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    let uri = format!("/orgs/{o}/members/{}", a.id);
    for b in [json!({"role":"coordinator"}), json!({"disabled":true})] {
        let (s, _, e) = call(&pool, "PATCH", &uri, &ck(&a.token), Some(b)).await;
        assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("LAST_OWNER")));
    }
}

/// 兩位 owner 同時互降：組織列鎖把兩個請求串行化，恰好一個成功、另一個 409。
/// 測試先持有組織列鎖，確認兩個請求都卡在鎖上，才放行，結果與排程無關。
#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_mutual_demotion_keeps_one_owner(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    member(&pool, o, b.id, "owner").await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *tx).await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE").bind(o).fetch_one(&mut *tx).await.unwrap();
    let patch = |uri: String, tok: String| {
        let app = app.clone();
        tokio::spawn(async move { call_app(&app, "PATCH", &uri, &ck(&tok), Some(json!({"role":"coordinator"}))).await })
    };
    let h1 = patch(format!("/orgs/{o}/members/{}", b.id), a.token.clone());
    let h2 = patch(format!("/orgs/{o}/members/{}", a.id), b.token.clone());
    // 兩個請求在組織列鎖上排隊（第二個等的是第一個，不是 fixture），所以數等鎖的連線
    wait_until("兩個 PATCH 都卡在組織列鎖", || async {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'")
            .fetch_one(&pool).await.unwrap();
        n >= 2
    }).await;
    tx.commit().await.unwrap();
    let (r1, r2) = (h1.await.unwrap(), h2.await.unwrap());
    let mut codes = [r1.0.as_u16(), r2.0.as_u16()];
    codes.sort();
    assert_eq!(codes, [200, 409], "{} {}", r1.2, r2.2);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM org_members WHERE org_id=$1 AND role='owner' AND disabled_at IS NULL").bind(o).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1, "org-last-owner lock missing: no owner-count guard");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn disabled_member_loses_access_immediately(pool: PgPool) {
    let a = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    member(&pool, o, c.id, "coordinator").await;
    let uri = format!("/orgs/{o}");
    assert_eq!(call(&pool, "GET", &uri, &ck(&c.token), None).await.0, StatusCode::OK);
    let (s, _, m) = call(&pool, "PATCH", &format!("{uri}/members/{}", c.id), &ck(&a.token), Some(json!({"disabled":true}))).await;
    assert_eq!((s, m["disabled_at"].is_null()), (StatusCode::OK, false));
    assert_eq!(call(&pool, "GET", &uri, &ck(&c.token), None).await.0, StatusCode::NOT_FOUND);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='org.member.update'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_owner_manages_members(pool: PgPool) {
    let a = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    member(&pool, o, c.id, "coordinator").await;
    let (s, v) = invite(&pool, o, &c.token, "b@x.com", "volunteer").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("ORG_ROLE_REQUIRED")));
    let (s, _, _) = call(&pool, "PATCH", &format!("/orgs/{o}/members/{}", a.id), &ck(&c.token), Some(json!({"role":"volunteer"}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
