//! OTP 登入、一次性驗證與錯誤次數限制。
mod common;
use axum::http::{header, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

/// 固定測試碼，不依賴 SMTP 或暴力枚舉。
async fn recover_code(pool: &PgPool, email: &str) -> String {
    let code = "123456";
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2 AND consumed_at IS NULL")
        .bind(wishly_api::auth::code_hash(
            code,
            &wishly_api::config::get().otp_pepper,
        ))
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
    code.to_string()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_full_flow(pool: PgPool) {
    let app = test_app(&pool);
    let email = format!("otp-{}@Example.com", Uuid::new_v4());
    let lower = email.to_lowercase();
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 202);
    assert_eq!(b["expires_in"], 600);
    let cid = b["challenge_id"].clone();
    // 60 秒內重送 → 429
    let (st, h, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 429);
    assert_eq!(b["code"], "RATE_LIMITED");
    assert!(h.contains_key("retry-after"));
    let code = recover_code(&pool, &lower).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": wrong, "challenge_id": cid })),
    )
    .await;
    assert_eq!(
        (st, b["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("OTP_INVALID"))
    );
    let (st, h, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code, "challenge_id": cid })),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["is_new_user"], true);
    assert_eq!(b["user"]["email"], lower);
    let sc = h
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        sc.starts_with("ws_session=") && sc.contains("HttpOnly") && sc.contains("SameSite=Lax"),
        "{sc}"
    );
    let cookie = sc.split(';').next().unwrap().to_string();
    // 驗證碼只能用一次
    let (st, ..) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code, "challenge_id": cid })),
    )
    .await;
    assert_eq!(st, 400);
    // /me
    let (st, _, me) = call_app(&app, "GET", "/me", &[("cookie", cookie.clone())], None).await;
    assert_eq!(st, 200);
    assert_eq!(me["identities"][0]["provider"], "email");
    assert_eq!(me["notification_prefs"], json!({}));
    let (st, _, me) = call_app(
        &app,
        "PATCH",
        "/me",
        &[("cookie", cookie.clone())],
        Some(json!({ "display_name": "小米", "notification_prefs": { "email_gifts": false } })),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(me["display_name"], "小米");
    assert_eq!(me["notification_prefs"]["email_gifts"], false);
    let (st, ..) = call_app(
        &app,
        "PATCH",
        "/me",
        &[("cookie", cookie.clone())],
        Some(json!({ "notification_prefs": { "evil": true } })),
    )
    .await;
    assert_eq!(st, 422);
    // 登出後 session 失效
    let (st, h, _) = call_app(
        &app,
        "POST",
        "/auth/logout",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(st, 204);
    assert!(h
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    let (st, ..) = call_app(&app, "GET", "/me", &[("cookie", cookie.clone())], None).await;
    assert_eq!(st, 401);
    // 再次登入同 email → 同一 user，非新使用者（先把上一張挑戰時間往前挪以略過 60 秒限制）
    sqlx::query(
        "UPDATE otp_challenges SET created_at = created_at - interval '2 minutes' WHERE email=$1",
    )
    .bind(&lower)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM rate_limits WHERE key LIKE 'otp_gap:%'")
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    let cid = b["challenge_id"].clone();
    let code = recover_code(&pool, &lower).await;
    let (_, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code, "challenge_id": cid })),
    )
    .await;
    assert_eq!(b["is_new_user"], false);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_locks_after_5_wrong_attempts(pool: PgPool) {
    let app = test_app(&pool);
    let email = format!("lock-{}@example.com", Uuid::new_v4());
    let (_, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    let cid = b["challenge_id"].clone();
    let code = recover_code(&pool, &email).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..5 {
        call_app(
            &app,
            "POST",
            "/auth/otp/verify",
            &[],
            Some(json!({ "email": email, "code": wrong, "challenge_id": cid })),
        )
        .await;
    }
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code, "challenge_id": cid })),
    )
    .await;
    assert_eq!(
        (st, b["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("OTP_INVALID"))
    );
    let (st, ..) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": "bad" })),
    )
    .await;
    assert_eq!(st, 422);
    assert_all_reconciled(&pool).await;
}

/// 匿名者用 login 用途耗盡額度，不得擋掉本人的 reset 發信與 reset 驗證。
#[sqlx::test(migrations = "../../db/migrations")]
async fn login_spend_does_not_lock_reset(pool: PgPool) {
    let app = test_app(&pool);
    let u = user(&pool, "victim").await;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(u.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    // 5 封 login 挑戰，各 2 次錯誤嘗試（共 10），皆已過 60 秒間隔
    for _ in 0..5 {
        sqlx::query("INSERT INTO otp_challenges (email, code_hash, expires_at, purpose, attempts, created_at) VALUES ($1, $2, now() + interval '5 minutes', 'login', 2, now() - interval '2 minutes')")
            .bind(&email).bind(vec![0u8; 32]).execute(&pool).await.unwrap();
    }
    let body = json!({ "email": email });
    let (st, ..) = call_app(
        &app,
        "POST",
        "/auth/password/reset/request",
        &[],
        Some(body),
    )
    .await;
    assert_eq!(st, 202);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_challenges WHERE email=$1 AND purpose='reset'",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "reset 發信不應被 login 額度吃掉");
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2 AND purpose='reset'")
        .bind(wishly_api::auth::code_hash(
            "123456",
            &wishly_api::config::get().otp_pepper,
        ))
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();
    let body = json!({ "email": email, "code": "123456", "new_password": "NewPassw0rd!x" });
    let (st, ..) = call_app(
        &app,
        "POST",
        "/auth/password/reset/confirm",
        &[],
        Some(body),
    )
    .await;
    assert_ne!(st, 429, "正確的 reset 碼不應被 login 嘗試數擋下");
}

/// 以指定來源 IP 呼叫（rate limit 以 IP 區分請求者）。
async fn from_ip(
    app: &axum::Router,
    ip: &str,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut req = request("POST", &format!("/api/v1{uri}"), &[], Some(body));
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
            format!("{ip}:1234").parse().unwrap(),
        ));
    let (st, _, b) = call_request(app, req).await;
    (st, b)
}

async fn set_code(pool: &PgPool, id: &str, code: &str) {
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE id=$2::uuid")
        .bind(wishly_api::auth::code_hash(
            code,
            &wishly_api::config::get().otp_pepper,
        ))
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

/// 其他 IP 的攻擊者大量請求 / 亂猜，不得擋住本人，也不得讓本人的碼失效。
#[sqlx::test(migrations = "../../db/migrations")]
async fn attacker_cannot_lock_out_victim_with_challenge_id(pool: PgPool) {
    let app = test_app(&pool);
    let email = format!("victim-{}@example.com", Uuid::new_v4());
    let req = json!({ "email": email });
    // 攻擊者（多個 IP）先要幾張、其後本人要一張、攻擊者再要一張更新的
    let mut atk = vec![];
    for i in 1..=3 {
        let (st, b) = from_ip(
            &app,
            &format!("10.0.0.{i}"),
            "/auth/otp/request",
            req.clone(),
        )
        .await;
        assert_eq!(st, 202);
        atk.push(b["challenge_id"].as_str().unwrap().to_string());
    }
    let (st, b) = from_ip(&app, "192.168.1.1", "/auth/otp/request", req.clone()).await;
    assert_eq!(st, 202, "本人不得被攻擊者的請求擋住：{b}");
    let vid = b["challenge_id"].as_str().unwrap().to_string();
    let (st, b) = from_ip(&app, "10.0.0.9", "/auth/otp/request", req.clone()).await;
    assert_eq!(st, 202);
    let newer = b["challenge_id"].as_str().unwrap().to_string();
    // 攻擊者對自己的挑戰亂猜 12 次（單張 5 次後即失效）
    for (i, id) in [&atk[0], &atk[1], &atk[2], &newer]
        .iter()
        .cycle()
        .take(12)
        .enumerate()
    {
        let (st, _) = from_ip(
            &app,
            &format!("10.0.1.{i}"),
            "/auth/otp/verify",
            json!({ "email": email, "code": "000000", "challenge_id": id }),
        )
        .await;
        assert_eq!(st, 400);
    }
    // 本人帶自己的 challenge_id 驗證成功，即使有更新的挑戰存在
    set_code(&pool, &vid, "123456").await;
    let (st, b) = from_ip(
        &app,
        "192.168.1.1",
        "/auth/otp/verify",
        json!({ "email": email, "code": "123456", "challenge_id": vid }),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_all_reconciled(&pool).await;
}

/// challenge_id 屬於別的 email / 用途 → 拒絕；匿名發出的挑戰不帶 challenge_id 也驗不到。
#[sqlx::test(migrations = "../../db/migrations")]
async fn challenge_id_is_bound_to_email_and_purpose(pool: PgPool) {
    let app = test_app(&pool);
    let a = format!("a-{}@example.com", Uuid::new_v4());
    let b = format!("b-{}@example.com", Uuid::new_v4());
    let (_, ra) = from_ip(&app, "10.1.0.1", "/auth/otp/request", json!({ "email": a })).await;
    let (_, rb) = from_ip(&app, "10.1.0.2", "/auth/otp/request", json!({ "email": b })).await;
    let (ida, idb) = (
        ra["challenge_id"].as_str().unwrap(),
        rb["challenge_id"].as_str().unwrap(),
    );
    set_code(&pool, ida, "123456").await;
    set_code(&pool, idb, "123456").await;
    // 用 b 的挑戰驗 a 的 email
    let (st, body) = from_ip(
        &app,
        "10.1.0.3",
        "/auth/otp/verify",
        json!({ "email": a, "code": "123456", "challenge_id": idb }),
    )
    .await;
    assert_eq!(
        (st, body["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("OTP_INVALID"))
    );
    // 用 login 挑戰走 register/verify、reset/confirm（用途不符）
    for (uri, extra) in [
        ("/auth/register/verify", json!({})),
        (
            "/auth/password/reset/confirm",
            json!({ "new_password": "NewPassw0rd!x" }),
        ),
    ] {
        let mut body = json!({ "email": a, "code": "123456", "challenge_id": ida });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (st, _) = from_ip(&app, "10.1.0.3", uri, body).await;
        assert_eq!(st, 400, "{uri}");
    }
    // 匿名端點發出的挑戰已綁定：不帶 challenge_id 驗證不到（他人無法不帶 id 燒掉本人那張）
    let (st, body) = from_ip(
        &app,
        "10.1.0.3",
        "/auth/otp/verify",
        json!({ "email": a, "code": "123456" }),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    // 本人帶對的 id 仍可驗證（上面不帶 id 的嘗試沒有燒掉這張）
    let (st, body) = from_ip(
        &app,
        "10.1.0.3",
        "/auth/otp/verify",
        json!({ "email": a, "code": "123456", "challenge_id": ida }),
    )
    .await;
    assert_eq!(st, 200, "{body}");
}

/// reset/request 對不存在與存在的帳號回應形狀一致（都含 challenge_id）。
#[sqlx::test(migrations = "../../db/migrations")]
async fn reset_request_shape_is_enumeration_safe(pool: PgPool) {
    let app = test_app(&pool);
    let u = user(&pool, "exists").await;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(u.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s1, b1) = from_ip(
        &app,
        "10.2.0.1",
        "/auth/password/reset/request",
        json!({ "email": email }),
    )
    .await;
    let (s2, b2) = from_ip(
        &app,
        "10.2.0.2",
        "/auth/password/reset/request",
        json!({ "email": "ghost@example.com" }),
    )
    .await;
    assert_eq!((s1, s2), (StatusCode::ACCEPTED, StatusCode::ACCEPTED));
    let keys = |v: &serde_json::Value| {
        v.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(keys(&b1), keys(&b2));
    assert!(Uuid::parse_str(b1["challenge_id"].as_str().unwrap()).is_ok());
    assert!(Uuid::parse_str(b2["challenge_id"].as_str().unwrap()).is_ok());
    // 不存在者不產生挑戰
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM otp_challenges WHERE email='ghost@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
}

/// 硬上限：每 (email, 用途) 每小時 20 封。
#[sqlx::test(migrations = "../../db/migrations")]
async fn hard_ceiling_is_20_per_hour(pool: PgPool) {
    let app = test_app(&pool);
    let email = "ceiling@example.com";
    sqlx::query("INSERT INTO otp_challenges (email, code_hash, expires_at, purpose, created_at) SELECT $1, '\\x00', now() + interval '5 minutes', 'login', now() - interval '2 minutes' FROM generate_series(1,20)")
        .bind(email)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _) = from_ip(
        &app,
        "10.3.0.1",
        "/auth/otp/request",
        json!({ "email": email }),
    )
    .await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
}
