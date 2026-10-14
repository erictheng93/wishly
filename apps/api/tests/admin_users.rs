//! 後台使用者生命週期、權限與稽核查詢的回歸測試。
mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishsync_api::session::hash_token;

async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (s, _, b) = common::api(
        pool,
        method,
        uri,
        &[("authorization", format!("Bearer {token}"))],
        body,
    )
    .await;
    (s, b)
}
async fn user(pool: &PgPool, staff: bool, name: &str) -> (Uuid, String, String) {
    let (id, token) = common::user_with_staff(pool, name, staff).await;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'email',$2,$2)").bind(id).bind(&email).execute(pool).await.unwrap();
    (id, token, email)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn suspension_revokes_sessions_blocks_login_and_is_audited(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (id, token, email) = user(&pool, false, "user").await;
    let path = format!("/admin/users/{id}");
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &path,
            &token,
            Some(json!({"blocked":true,"reason":"abuse"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &path,
            &staff,
            Some(json!({"blocked":true,"reason":" "}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, b) = call(
        &pool,
        "PATCH",
        &path,
        &staff,
        Some(json!({"blocked":true,"reason":"abuse"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["blocked"], true);
    assert_eq!(
        call(&pool, "GET", "/me", &token, None).await.0,
        StatusCode::FORBIDDEN
    );
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM sessions WHERE token_hash=$1")
            .bind(hash_token(&token))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked);
    // OTP 簽發與 OAuth 都經 new_session；測 OTP 不可重建登入。
    sqlx::query("INSERT INTO otp_challenges(email,code_hash,expires_at) VALUES ($1,$2,now()+interval '10 minutes')")
        .bind(&email).bind(wishsync_api::auth::code_hash("123456", &wishsync_api::config::get().otp_pepper)).execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &pool,
            "POST",
            "/auth/otp/verify",
            "",
            Some(json!({"email":email,"code":"123456"}))
        )
        .await
        .1["code"],
        "ACCOUNT_BLOCKED"
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &path,
            &staff,
            Some(json!({"blocked":false,"reason":"appeal"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&pool, "GET", "/me", &token, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, audit) = call(
        &pool,
        "GET",
        &format!("/admin/audit-logs?entity_id={id}&action=user.update&limit=1"),
        &staff,
        None,
    )
    .await;
    assert_eq!(audit["data"].as_array().unwrap().len(), 1);
    assert!(audit["next_cursor"].is_string());
    let next = audit["next_cursor"].as_str().unwrap();
    let (_, page) = call(
        &pool,
        "GET",
        &format!("/admin/audit-logs?entity_id={id}&action=user.update&limit=1&cursor={next}"),
        &staff,
        None,
    )
    .await;
    assert_ne!(audit["data"][0]["id"], page["data"][0]["id"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn last_staff_and_last_login_are_protected(pool: PgPool) {
    let (sid, staff, _) = user(&pool, true, "staff").await;
    for body in [
        json!({"is_staff":false,"reason":"demote"}),
        json!({"blocked":true,"reason":"block"}),
    ] {
        assert_eq!(
            call(
                &pool,
                "PATCH",
                &format!("/admin/users/{sid}"),
                &staff,
                Some(body)
            )
            .await
            .1["code"],
            "LAST_STAFF"
        );
    }
    let (id, token, _) = user(&pool, false, "user").await;
    let path = format!("/admin/users/{id}/identities/email");
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &path,
            &staff,
            Some(json!({"reason":"help"}))
        )
        .await
        .1["code"],
        "LAST_IDENTITY"
    );
    sqlx::query("INSERT INTO auth_identities(user_id,provider,provider_uid) VALUES ($1,'line',$2)")
        .bind(id)
        .bind(id.to_string())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/admin/users/{id}/identities/line"),
            &staff,
            Some(json!({"reason":"help"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&pool, "GET", "/me", &token, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/admin/users/{id}"),
            &staff,
            Some(json!({"is_staff":true,"reason":"hire"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/admin/users/{sid}"),
            &staff,
            Some(json!({"is_staff":false,"reason":"handover"}))
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn name_provider_filters_stats_and_recovery(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (id, _, _) = user(&pool, false, "LINE小明").await;
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET email=NULL,handle='line_ming' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities(user_id,provider,provider_uid) VALUES ($1,'line',$2)")
        .bind(id)
        .bind(id.to_string())
        .execute(&pool)
        .await
        .unwrap();
    for q in ["q=LINE", "q=line_ming", "provider=line&status=active"] {
        let (s, b) = call(&pool, "GET", &format!("/admin/users?{q}"), &staff, None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b["data"][0]["id"], id.to_string());
        assert_eq!(b["data"][0]["identities"][0]["provider"], "line");
    }
    assert_eq!(
        call(&pool, "GET", "/admin/users?provider=bogus", &staff, None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/admin/users/{id}/recovery"),
            &staff,
            Some(json!({"reason":"help"}))
        )
        .await
        .1["code"],
        "RECOVERY_UNAVAILABLE"
    );
    let (eid, _, email) = user(&pool, false, "email").await;
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/admin/users/{eid}/recovery"),
            &staff,
            Some(json!({"reason":"help"}))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_challenges WHERE email=$1 AND purpose='reset'",
    )
    .bind(email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    let (_, stats) = call(&pool, "GET", "/admin/stats", &staff, None).await;
    assert_eq!(stats["users"], 3);
    assert_eq!(
        call(
            &pool,
            "GET",
            "/admin/audit-logs?from=2026-10-12&to=2026-10-11",
            &staff,
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(&pool, "GET", "/admin/wallets?status=bogus", &staff, None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(&pool, "GET", "/admin/wishlists?status=bogus", &staff, None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_staff_changes_keep_an_effective_staff(pool: PgPool) {
    let (a, ta, _) = user(&pool, true, "a").await;
    let (b, tb, _) = user(&pool, true, "b").await;
    let body = json!({"is_staff":false,"reason":"handover"});
    let pa = format!("/admin/users/{a}");
    let pb = format!("/admin/users/{b}");
    let (ra, rb) = tokio::join!(
        call(&pool, "PATCH", &pa, &ta, Some(body.clone())),
        call(&pool, "PATCH", &pb, &tb, Some(body))
    );
    assert_eq!(
        [ra.0, rb.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE is_staff AND blocked_at IS NULL AND deleted_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    let token = if ra.0 == StatusCode::OK { tb } else { ta };
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            &token,
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .1["code"],
        "LAST_STAFF"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn filters_paginate_and_sensitive_routes_require_staff(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (a, token, _) = user(&pool, false, "A").await;
    let (b, _, _) = user(&pool, false, "B").await;
    let (s, p) = call(
        &pool,
        "GET",
        "/admin/users?status=active&provider=email&limit=1",
        &staff,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["data"].as_array().unwrap().len(), 1);
    let (_, p2) = call(
        &pool,
        "GET",
        &format!(
            "/admin/users?status=active&provider=email&limit=1&cursor={}",
            p["next_cursor"].as_str().unwrap()
        ),
        &staff,
        None,
    )
    .await;
    assert_ne!(p["data"][0]["id"], p2["data"][0]["id"]);
    sqlx::query("INSERT INTO point_wallets(user_id,status) VALUES ($1,'frozen')")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    let (_, frozen) = call(&pool, "GET", "/admin/wallets?status=frozen", &staff, None).await;
    assert_eq!(frozen["data"][0]["user"]["id"], a.to_string());
    let (_, none) = call(&pool, "GET", "/admin/wallets?status=none", &staff, None).await;
    assert!(none["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["user"]["id"] == b.to_string()));
    for uri in [
        "/admin/stats",
        "/admin/audit-logs",
        "/admin/users",
        "/admin/system-flags",
    ] {
        assert_eq!(
            call(&pool, "GET", uri, &token, None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&pool, "GET", uri, "", None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_unlink_clears_provider_email_and_notifies_user(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (id, _, email) = user(&pool, false, "user").await;
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities(user_id,provider,provider_uid,email) VALUES ($1,'google',$2,$3),($1,'line',$4,NULL)")
        .bind(id).bind(format!("g-{id}")).bind(&email).bind(format!("l-{id}")).execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/admin/users/{id}/identities/google"),
            &staff,
            Some(json!({"reason":"lost access"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let contact: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        contact.is_none(),
        "解除 Google 不可保留該 Google 提供的 Email 登入捷徑"
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='auth.identity_unlinked'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    let (next, _) = wishsync_api::auth::upsert_identity(
        &pool,
        "google",
        &format!("g-{id}"),
        Some(&email),
        "again",
        None,
    )
    .await
    .unwrap();
    assert_ne!(next, id);
    let (s, body) = call(
        &pool,
        "DELETE",
        &format!("/admin/users/{id}/identities/line"),
        &staff,
        Some(json!({"reason":"help"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LAST_IDENTITY");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn audit_date_range_uses_taipei_inclusive_end_day(pool: PgPool) {
    let (sid, staff, _) = user(&pool, true, "staff").await;
    let (id, _, _) = user(&pool, false, "target").await;
    for (time, n) in [
        ("2026-10-11T15:59:59Z", 0),
        ("2026-10-11T16:00:00Z", 1),
        ("2026-10-12T15:59:59Z", 2),
        ("2026-10-12T16:00:00Z", 3),
    ] {
        sqlx::query("INSERT INTO audit_logs(actor_type,actor_id,action,entity,entity_id,diff,created_at) VALUES ('staff',$1,'test.audit','users',$2,$3,$4::text::timestamptz)")
            .bind(sid).bind(id).bind(json!({"n":n})).bind(time).execute(&pool).await.unwrap();
    }
    let (s,b) = call(&pool,"GET",&format!("/admin/audit-logs?actor_id={sid}&entity=users&entity_id={id}&action=test.audit&from=2026-10-12&to=2026-10-12"),&staff,None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["data"].as_array().unwrap().len(), 2);
    let mut ns: Vec<i64> = b["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["diff"]["n"].as_i64().unwrap())
        .collect();
    ns.sort();
    assert_eq!(ns, vec![1, 2]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn recovery_uses_one_connection_and_rolls_back_on_rate_limit(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (id, _, email) = user(&pool, false, "target").await;
    let single = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let path = format!("/admin/users/{id}/recovery");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        call(
            &single,
            "POST",
            &path,
            &staff,
            Some(json!({"reason":"help"})),
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.0, StatusCode::ACCEPTED, "{result:?}");
    assert_eq!(
        call(
            &single,
            "POST",
            &path,
            &staff,
            Some(json!({"reason":"retry"}))
        )
        .await
        .1["code"],
        "RATE_LIMITED"
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_challenges WHERE email=$1 AND purpose='reset'",
    )
    .bind(email)
    .fetch_one(&single)
    .await
    .unwrap();
    assert_eq!(n, 1);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity_id=$1 AND action='user.recovery'",
    )
    .bind(id)
    .fetch_one(&single)
    .await
    .unwrap();
    assert_eq!(n, 1, "OTP 與稽核須一同提交或回滾");
    single.close().await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_blocks_admin_suspension_without_side_effects(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (id, token, _) = user(&pool, false, "target").await;
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &staff,
            Some(json!({"value":true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let path = format!("/admin/users/{id}");
    let (status, body) = call(
        &pool,
        "PATCH",
        &path,
        &staff,
        Some(json!({"blocked":true,"reason":"abuse"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "READ_ONLY_MODE");
    let blocked: bool = sqlx::query_scalar("SELECT blocked_at IS NOT NULL FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!blocked);
    assert_eq!(
        call(&pool, "GET", "/me", &token, None).await.0,
        StatusCode::OK
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity_id=$1 AND action='user.update'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &staff,
            Some(json!({"value":false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &path,
            &staff,
            Some(json!({"blocked":true,"reason":"abuse"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&pool, "GET", "/me", &token, None).await.1["code"],
        "ACCOUNT_BLOCKED"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn recovery_rechecks_email_after_waiting_for_otp_lock(pool: PgPool) {
    let (_, staff, _) = user(&pool, true, "staff").await;
    let (id, _, email) = user(&pool, false, "target").await;
    let mut change = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("otp:{email}"))
        .execute(&mut *change)
        .await
        .unwrap();
    let recovery = tokio::spawn({
        let pool = pool.clone();
        async move {
            call(
                &pool,
                "POST",
                &format!("/admin/users/{id}/recovery"),
                &staff,
                Some(json!({"reason":"help"})),
            )
            .await
        }
    });
    common::wait_until("recovery advisory lock wait", || async {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE a.datname=current_database() AND l.locktype='advisory' AND NOT l.granted)")
            .fetch_one(&pool).await.unwrap()
    }).await;
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1")
        .bind(id)
        .execute(&mut *change)
        .await
        .unwrap();
    change.commit().await.unwrap();
    let (status, body) = recovery.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "RECOVERY_UNAVAILABLE");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM otp_challenges WHERE email=$1")
        .bind(email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "舊信箱的 OTP 必須回滾");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity_id=$1 AND action='user.recovery'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
}
