//! SESSION_CACHE：成功的登入驗證在程序內快取，撤銷 / 停權 / MFA 變更由 DB trigger（0065）經 LISTEN 立即清空。
//! config 與快取是全域單例，所以獨立成一個 binary、只有一個測試函式。
mod common;
use axum::http::StatusCode;
use common::*;
use sqlx::PgPool;
use std::time::{Duration, Instant};

async fn me(pool: &PgPool, token: &str) -> (StatusCode, serde_json::Value) {
    let (s, _, v) = call(pool, "GET", "/me", &ck(token), None).await;
    (s, v)
}

/// 失效是非同步的（NOTIFY 在 commit 後送達 listener），最多等 3 秒
async fn eventually(
    pool: &PgPool,
    token: &str,
    label: &str,
    ok: impl Fn(StatusCode, &serde_json::Value) -> bool,
) {
    let t = Instant::now();
    loop {
        let (s, v) = me(pool, token).await;
        if ok(s, &v) {
            return;
        }
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "{label}: 3 秒內未失效（{s} {v}）"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn session_cache_hits_and_is_invalidated_by_triggers(pool: PgPool) {
    wishly_api::config::init(wishly_api::config::Config {
        session_cache: true,
        ..wishly_api::config::Config::dev()
    });
    assert!(
        !wishly_api::session::cache_active(),
        "LISTEN 連上前不得使用快取"
    );
    wishly_api::session::spawn_cache_listener(pool.clone());
    let t = Instant::now();
    while !wishly_api::session::cache_active() {
        assert!(t.elapsed() < Duration::from_secs(5), "listener 未連上");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 1) 確實有快取：關掉 trigger 後在 DB 撤銷，快取仍放行（證明命中的是快取）
    let a = user(&pool, "a").await;
    assert_eq!(me(&pool, &a.token).await.0, StatusCode::OK);
    sqlx::query("ALTER TABLE sessions DISABLE TRIGGER sessions_cache_inval_upd")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET revoked_at = now() - interval '1 second' WHERE user_id = $1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me(&pool, &a.token).await.0, StatusCode::OK, "應命中快取");
    // trigger 恢復後，任何撤銷類變更都會清空快取
    sqlx::query("ALTER TABLE sessions ENABLE TRIGGER sessions_cache_inval_upd")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    eventually(&pool, &a.token, "撤銷", |s, _| {
        s == StatusCode::UNAUTHORIZED
    })
    .await;

    // 2) 登出（API 路徑）
    let b = user(&pool, "b").await;
    assert_eq!(me(&pool, &b.token).await.0, StatusCode::OK);
    assert_eq!(
        call(&pool, "POST", "/auth/logout", &ck(&b.token), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    eventually(&pool, &b.token, "登出", |s, _| {
        s == StatusCode::UNAUTHORIZED
    })
    .await;

    // 3) 停權（users trigger）：回 403 ACCOUNT_BLOCKED
    let c = user(&pool, "c").await;
    assert_eq!(me(&pool, &c.token).await.0, StatusCode::OK);
    sqlx::query("UPDATE users SET blocked_at = now() WHERE id = $1")
        .bind(c.id)
        .execute(&pool)
        .await
        .unwrap();
    eventually(&pool, &c.token, "停權", |s, v| {
        s == StatusCode::FORBIDDEN && v["code"] == "ACCOUNT_BLOCKED"
    })
    .await;

    // 4) 通過 MFA（mfa_at 變更）：快取中的 mfa_verified=false 要更新
    let d = user(&pool, "d").await;
    assert_eq!(me(&pool, &d.token).await.1["mfa_verified"], false);
    sqlx::query("UPDATE sessions SET mfa_at = now() WHERE user_id = $1")
        .bind(d.id)
        .execute(&pool)
        .await
        .unwrap();
    eventually(&pool, &d.token, "MFA", |s, v| {
        s == StatusCode::OK && v["mfa_verified"] == true
    })
    .await;

    // 5) 刪除 session 列
    let e = user(&pool, "e").await;
    assert_eq!(me(&pool, &e.token).await.0, StatusCode::OK);
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(e.id)
        .execute(&pool)
        .await
        .unwrap();
    eventually(&pool, &e.token, "刪除 session", |s, _| {
        s == StatusCode::UNAUTHORIZED
    })
    .await;

    // 6) 每 5 分鐘的續期（last_seen_at、延長 expires_at）不得發通知，否則快取形同虛設；撤銷則要發
    let f = user(&pool, "f").await;
    let mut l = sqlx::postgres::PgListener::connect_with(&pool)
        .await
        .unwrap();
    l.listen("session_inval").await.unwrap();
    sqlx::query("UPDATE sessions SET last_seen_at = now(), expires_at = expires_at + interval '1 day' WHERE user_id = $1")
        .bind(f.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), l.recv())
            .await
            .is_err(),
        "續期不應發通知"
    );
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1")
        .bind(f.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), l.recv())
            .await
            .is_ok(),
        "撤銷應發通知"
    );
}
