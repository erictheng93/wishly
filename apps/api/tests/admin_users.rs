//! 後台使用者生命週期、權限與稽核查詢的回歸測試。
mod common;
use axum::http::StatusCode;
use common::{call, ck, staff, user, wishlist, ListOpts, TestList, TestUser};
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
            "LAST_ADMIN"
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
            Some(json!({"staff_roles":["admin"],"reason":"hire"}))
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
        "LAST_ADMIN"
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
    let self_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='auth.identity_unlinked'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        self_rows, 0,
        "營運代為解除只記 user.identity_unlink，不冒充本人"
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_search_audit_masks_email_and_phone(pool: PgPool) {
    let (sid, staff, _) = email_user(&pool, true, "staff").await;
    for (path, q, want) in [
        ("/admin/users", "alice@example.com", "a***@example.com"),
        ("/admin/users", "0912-345-678", "***678"),
        ("/admin/users", "小明", "小明"),
        ("/admin/wallets", "bob@example.com", "b***@example.com"),
    ] {
        let (s, _, b) = call(
            &pool,
            "GET",
            &format!("{path}?q={}", urlencoding(q)),
            &[("authorization", format!("Bearer {staff}"))],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let got: String = sqlx::query_scalar("SELECT diff->>'q' FROM audit_logs WHERE actor_id=$1 AND action='admin.search' ORDER BY id DESC LIMIT 1")
            .bind(sid).fetch_one(&pool).await.unwrap();
        assert_eq!(got, want);
    }
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
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

#[sqlx::test(migrations = "../../db/migrations")]
async fn blocked_owner_content_is_not_found_and_purged(pool: PgPool) {
    let cdn = common::cdn::init();
    let TestUser { id: owner, .. } = user(&pool, "Owner").await;
    let TestUser { token: staff, .. } = staff(&pool, "Staff").await;
    sqlx::query("UPDATE users SET handle='blk_owner' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let TestList { slug, .. } = wishlist(&pool, owner, ListOpts::default()).await;
    let list = format!("/public/wishlists/{slug}");
    let patch = |blocked: bool| json!({"blocked":blocked,"reason":"abuse"});
    assert_eq!(call(&pool, "GET", &list, &[], None).await.0, StatusCode::OK);
    assert_eq!(
        call(&pool, "GET", "/users/blk_owner", &[], None).await.0,
        StatusCode::OK
    );
    let (s, _, r) = call(
        &pool,
        "PATCH",
        &format!("/admin/users/{owner}"),
        &ck(&staff),
        Some(patch(true)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    cdn.wait_for(&slug).await;
    assert_eq!(
        call(&pool, "GET", &list, &[], None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&pool, "GET", "/users/blk_owner", &[], None).await.0,
        StatusCode::NOT_FOUND
    );
    let (s, _, r) = call(
        &pool,
        "PATCH",
        &format!("/admin/users/{owner}"),
        &ck(&staff),
        Some(patch(false)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(call(&pool, "GET", &list, &[], None).await.0, StatusCode::OK);
    assert_eq!(
        call(&pool, "GET", "/users/blk_owner", &[], None).await.0,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_roles_gate_admin_endpoints(pool: PgPool) {
    let admin = staff(&pool, "admin").await;
    let fin = staff(&pool, "fin").await;
    let target = user(&pool, "target").await;
    let bearer = |t: &str| [("authorization", format!("Bearer {t}"))];
    // admin 把 fin 改成只有 finance；未知角色 422
    let path = format!("/admin/users/{}", fin.id);
    let (st, _, body) = call(
        &pool,
        "PATCH",
        &path,
        &bearer(&admin.token),
        Some(json!({"staff_roles":["finance","finance"],"reason":"r"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["staff_roles"], json!(["finance"]));
    assert_eq!(body["is_staff"], true);
    let (st, _, _) = call(
        &pool,
        "PATCH",
        &path,
        &bearer(&admin.token),
        Some(json!({"staff_roles":["root"],"reason":"r"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    // 改角色會撤 session？不會（is_staff 沒變），角色由快取失效即時生效
    let (st, _, me) = call(&pool, "GET", "/me", &bearer(&fin.token), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(me["staff_roles"], json!(["finance"]));
    // finance：金流可用、審核 / 系統旗標 / 改角色 / 停權不可用、共用唯讀可用
    for (m, p, ok) in [
        ("GET", "/admin/finance/overview", true),
        ("GET", "/admin/topups", true),
        ("GET", "/admin/users", true),
        ("GET", "/admin/reports", false),
        ("GET", "/admin/orgs", false),
        ("GET", "/admin/system-flags", false),
    ] {
        let (st, _, b) = call(&pool, m, p, &bearer(&fin.token), None).await;
        if ok {
            assert_eq!(st, StatusCode::OK, "{p}: {b}");
        } else {
            assert_eq!(st, StatusCode::FORBIDDEN, "{p}");
            assert_eq!(b["code"], "STAFF_ROLE_REQUIRED", "{p}");
        }
    }
    let tpath = format!("/admin/users/{}", target.id);
    for body in [
        json!({"blocked":true,"reason":"r"}),
        json!({"staff_roles":["admin"],"reason":"r"}),
    ] {
        let (st, _, b) = call(&pool, "PATCH", &tpath, &bearer(&fin.token), Some(body)).await;
        assert_eq!(
            (st, b["code"].clone()),
            (StatusCode::FORBIDDEN, json!("STAFF_ROLE_REQUIRED"))
        );
    }
    // moderator 可停權一般使用者，但不能停權營運人員
    let (st, _, _) = call(
        &pool,
        "PATCH",
        &path,
        &bearer(&admin.token),
        Some(json!({"staff_roles":["moderator"],"reason":"r"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(
        &pool,
        "PATCH",
        &tpath,
        &bearer(&fin.token),
        Some(json!({"blocked":true,"reason":"r"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let apath = format!("/admin/users/{}", admin.id);
    let (st, _, b) = call(
        &pool,
        "PATCH",
        &apath,
        &bearer(&fin.token),
        Some(json!({"blocked":true,"reason":"r"})),
    )
    .await;
    assert_eq!(
        (st, b["code"].clone()),
        (StatusCode::FORBIDDEN, json!("STAFF_ROLE_REQUIRED"))
    );
    // 唯一 admin 不可拿掉自己的 admin
    let (st, _, b) = call(
        &pool,
        "PATCH",
        &apath,
        &bearer(&admin.token),
        Some(json!({"staff_roles":["finance"],"reason":"r"})),
    )
    .await;
    assert_eq!(
        (st, b["code"].clone()),
        (StatusCode::CONFLICT, json!("LAST_ADMIN"))
    );
    // 舊寫法 is_staff=false 清空角色
    let (st, _, b) = call(
        &pool,
        "PATCH",
        &path,
        &bearer(&admin.token),
        Some(json!({"is_staff":false,"reason":"r"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        (b["is_staff"].clone(), b["staff_roles"].clone()),
        (json!(false), json!([]))
    );
}
