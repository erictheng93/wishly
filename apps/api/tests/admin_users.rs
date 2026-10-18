//! 後台使用者生命週期、權限與稽核查詢的回歸測試。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::session::hash_token;

async fn email_user(pool: &PgPool, staff: bool, name: &str) -> (Uuid, String, String) {
    let common::TestUser { id, token } = if staff {
        common::staff(pool, name).await
    } else {
        common::user(pool, name).await
    };
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
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (id, token, email) = email_user(&pool, false, "user").await;
    let path = format!("/admin/users/{id}");
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &path,
            &[("authorization", format!("Bearer {token}"))],
            Some(json!({"blocked":true,"reason":"abuse"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &path,
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"blocked":true,"reason":" "}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, b) = common::call(
        &pool,
        "PATCH",
        &path,
        &[("authorization", format!("Bearer {staff}"))],
        Some(json!({"blocked":true,"reason":"abuse"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["blocked"], true);
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/me",
            &[("authorization", format!("Bearer {token}"))],
            None
        )
        .await
        .0,
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
        .bind(&email).bind(wishly_api::auth::code_hash("123456", &wishly_api::config::get().otp_pepper)).execute(&pool).await.unwrap();
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/auth/otp/verify",
            &[("authorization", format!("Bearer {}", ""))],
            Some(json!({"email":email,"code":"123456"}))
        )
        .await
        .2["code"],
        "ACCOUNT_BLOCKED"
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &path,
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"blocked":false,"reason":"appeal"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/me",
            &[("authorization", format!("Bearer {token}"))],
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, _, audit) = common::call(
        &pool,
        "GET",
        &format!("/admin/audit-logs?entity_id={id}&action=user.update&limit=1"),
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
    assert_eq!(audit["data"].as_array().unwrap().len(), 1);
    assert!(audit["next_cursor"].is_string());
    let next = audit["next_cursor"].as_str().unwrap();
    let (_, _, page) = common::call(
        &pool,
        "GET",
        &format!("/admin/audit-logs?entity_id={id}&action=user.update&limit=1&cursor={next}"),
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
    assert_ne!(audit["data"][0]["id"], page["data"][0]["id"]);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn last_staff_and_last_login_are_protected(pool: PgPool) {
    let (sid, staff, _) = email_user(&pool, true, "staff").await;
    for body in [
        json!({"is_staff":false,"reason":"demote"}),
        json!({"blocked":true,"reason":"block"}),
    ] {
        assert_eq!(
            common::call(
                &pool,
                "PATCH",
                &format!("/admin/users/{sid}"),
                &[("authorization", format!("Bearer {staff}"))],
                Some(body)
            )
            .await
            .2["code"],
            "LAST_STAFF"
        );
    }
    let (id, token, _) = email_user(&pool, false, "user").await;
    let path = format!("/admin/users/{id}/identities/email");
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &path,
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"reason":"help"}))
        )
        .await
        .2["code"],
        "LAST_IDENTITY"
    );
    sqlx::query("INSERT INTO auth_identities(user_id,provider,provider_uid) VALUES ($1,'line',$2)")
        .bind(id)
        .bind(id.to_string())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &format!("/admin/users/{id}/identities/line"),
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"reason":"help"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/me",
            &[("authorization", format!("Bearer {token}"))],
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &format!("/admin/users/{id}"),
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"is_staff":true,"reason":"hire"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &format!("/admin/users/{sid}"),
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"is_staff":false,"reason":"handover"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn name_provider_filters_stats_and_recovery(pool: PgPool) {
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (id, _, _) = email_user(&pool, false, "LINE小明").await;
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
        let (s, _, b) = common::call(
            &pool,
            "GET",
            &format!("/admin/users?{q}"),
            &[("authorization", format!("Bearer {staff}"))],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b["data"][0]["id"], id.to_string());
        assert_eq!(b["data"][0]["identities"][0]["provider"], "line");
    }
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/admin/users?provider=bogus",
            &[("authorization", format!("Bearer {staff}"))],
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/admin/users/{id}/recovery"),
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"reason":"help"}))
        )
        .await
        .2["code"],
        "RECOVERY_UNAVAILABLE"
    );
    let (eid, _, email) = email_user(&pool, false, "email").await;
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/admin/users/{eid}/recovery"),
            &[("authorization", format!("Bearer {staff}"))],
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
    let (_, _, stats) = common::call(
        &pool,
        "GET",
        "/admin/stats",
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
    assert_eq!(stats["users"], 3);
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/admin/audit-logs?from=2026-10-12&to=2026-10-11",
            &[("authorization", format!("Bearer {staff}"))],
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/admin/wallets?status=bogus",
            &[("authorization", format!("Bearer {staff}"))],
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/admin/wishlists?status=bogus",
            &[("authorization", format!("Bearer {staff}"))],
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_staff_changes_keep_an_effective_staff(pool: PgPool) {
    let (a, ta, _) = email_user(&pool, true, "a").await;
    let (b, tb, _) = email_user(&pool, true, "b").await;
    let body = json!({"is_staff":false,"reason":"handover"});
    let pa = format!("/admin/users/{a}");
    let pb = format!("/admin/users/{b}");
    let (ra, rb) = tokio::join!(
        async {
            common::call(
                &pool,
                "PATCH",
                &pa,
                &[("authorization", format!("Bearer {ta}"))],
                Some(body.clone()),
            )
            .await
        },
        async {
            common::call(
                &pool,
                "PATCH",
                &pb,
                &[("authorization", format!("Bearer {tb}"))],
                Some(body.clone()),
            )
            .await
        }
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
        common::call(
            &pool,
            "DELETE",
            "/me",
            &[("authorization", format!("Bearer {token}"))],
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .2["code"],
        "LAST_STAFF"
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn filters_paginate_and_sensitive_routes_require_staff(pool: PgPool) {
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (a, token, _) = email_user(&pool, false, "A").await;
    let (b, _, _) = email_user(&pool, false, "B").await;
    let (s, _, p) = common::call(
        &pool,
        "GET",
        "/admin/users?status=active&provider=email&limit=1",
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["data"].as_array().unwrap().len(), 1);
    let (_, _, p2) = common::call(
        &pool,
        "GET",
        &format!(
            "/admin/users?status=active&provider=email&limit=1&cursor={}",
            p["next_cursor"].as_str().unwrap()
        ),
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
    assert_ne!(p["data"][0]["id"], p2["data"][0]["id"]);
    sqlx::query("INSERT INTO point_wallets(user_id,status) VALUES ($1,'frozen')")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, frozen) = common::call(
        &pool,
        "GET",
        "/admin/wallets?status=frozen",
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
    assert_eq!(frozen["data"][0]["user"]["id"], a.to_string());
    let (_, _, none) = common::call(
        &pool,
        "GET",
        "/admin/wallets?status=none",
        &[("authorization", format!("Bearer {staff}"))],
        None,
    )
    .await;
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
            common::call(
                &pool,
                "GET",
                uri,
                &[("authorization", format!("Bearer {token}"))],
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            common::call(
                &pool,
                "GET",
                uri,
                &[("authorization", format!("Bearer {}", ""))],
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_unlink_clears_provider_email_and_notifies_user(pool: PgPool) {
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (id, _, email) = email_user(&pool, false, "user").await;
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities(user_id,provider,provider_uid,email) VALUES ($1,'google',$2,$3),($1,'line',$4,NULL)")
        .bind(id).bind(format!("g-{id}")).bind(&email).bind(format!("l-{id}")).execute(&pool).await.unwrap();
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &format!("/admin/users/{id}/identities/google"),
            &[("authorization", format!("Bearer {staff}"))],
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
    let (next, _) = wishly_api::auth::upsert_identity(
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
    let (s, _, body) = common::call(
        &pool,
        "DELETE",
        &format!("/admin/users/{id}/identities/line"),
        &[("authorization", format!("Bearer {staff}"))],
        Some(json!({"reason":"help"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LAST_IDENTITY");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn audit_date_range_uses_taipei_inclusive_end_day(pool: PgPool) {
    let (sid, staff, _) = email_user(&pool, true, "staff").await;
    let (id, _, _) = email_user(&pool, false, "target").await;
    for (time, n) in [
        ("2026-10-11T15:59:59Z", 0),
        ("2026-10-11T16:00:00Z", 1),
        ("2026-10-12T15:59:59Z", 2),
        ("2026-10-12T16:00:00Z", 3),
    ] {
        sqlx::query("INSERT INTO audit_logs(actor_type,actor_id,action,entity,entity_id,diff,created_at) VALUES ('staff',$1,'test.audit','users',$2,$3,$4::text::timestamptz)")
            .bind(sid).bind(id).bind(json!({"n":n})).bind(time).execute(&pool).await.unwrap();
    }
    let (s, _, b) = common::call(&pool, "GET", &format!("/admin/audit-logs?actor_id={sid}&entity=users&entity_id={id}&action=test.audit&from=2026-10-12&to=2026-10-12"), &[("authorization", format!("Bearer {staff}"))], None).await;
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
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn recovery_uses_one_connection_and_rolls_back_on_rate_limit(pool: PgPool) {
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (id, _, email) = email_user(&pool, false, "target").await;
    let single = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    // 硬上限為每 (email, 用途) 每小時 20 封：先塞 19 封，第 20 封成功、第 21 封被擋
    sqlx::query("INSERT INTO otp_challenges (email, code_hash, expires_at, purpose, created_at) SELECT $1, '\\x00', now() + interval '5 minutes', 'reset', now() - interval '2 minutes' FROM generate_series(1,19)")
        .bind(&email).execute(&pool).await.unwrap();
    let path = format!("/admin/users/{id}/recovery");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        common::call(
            &single,
            "POST",
            &path,
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"reason":"help"})),
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.0, StatusCode::ACCEPTED, "{result:?}");
    assert_eq!(
        common::call(
            &single,
            "POST",
            &path,
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"reason":"retry"}))
        )
        .await
        .2["code"],
        "RATE_LIMITED"
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_challenges WHERE email=$1 AND purpose='reset'",
    )
    .bind(email)
    .fetch_one(&single)
    .await
    .unwrap();
    assert_eq!(n, 20);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity_id=$1 AND action='user.recovery'",
    )
    .bind(id)
    .fetch_one(&single)
    .await
    .unwrap();
    assert_eq!(n, 1, "OTP 與稽核須一同提交或回滾");
    single.close().await;
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_blocks_admin_suspension_without_side_effects(pool: PgPool) {
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (id, token, _) = email_user(&pool, false, "target").await;
    assert_eq!(
        common::call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"value":true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let path = format!("/admin/users/{id}");
    let (status, _, body) = common::call(
        &pool,
        "PATCH",
        &path,
        &[("authorization", format!("Bearer {staff}"))],
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
        common::call(
            &pool,
            "GET",
            "/me",
            &[("authorization", format!("Bearer {token}"))],
            None
        )
        .await
        .0,
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
        common::call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"value":false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &path,
            &[("authorization", format!("Bearer {staff}"))],
            Some(json!({"blocked":true,"reason":"abuse"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/me",
            &[("authorization", format!("Bearer {token}"))],
            None
        )
        .await
        .2["code"],
        "ACCOUNT_BLOCKED"
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn recovery_rechecks_email_after_waiting_for_otp_lock(pool: PgPool) {
    let (_, staff, _) = email_user(&pool, true, "staff").await;
    let (id, _, email) = email_user(&pool, false, "target").await;
    let mut change = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("otp:{email}"))
        .execute(&mut *change)
        .await
        .unwrap();
    let recovery = tokio::spawn({
        let pool = pool.clone();
        async move {
            common::call(
                &pool,
                "POST",
                &format!("/admin/users/{id}/recovery"),
                &[("authorization", format!("Bearer {staff}"))],
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
    let (status, _, body) = recovery.await.unwrap();
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
    common::assert_all_reconciled(&pool).await;
}
