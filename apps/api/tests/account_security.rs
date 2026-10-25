//! F-08 刪帳號取消捐助、F-12 problem+json、F-18 安全標頭、F-24 登入鎖定、F-26 退訂 POST
mod common;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

fn with_peer(mut req: Request<Body>, ip: Option<&str>) -> Request<Body> {
    if let Some(ip) = ip {
        req.extensions_mut().insert(ConnectInfo(
            format!("{ip}:1234").parse::<SocketAddr>().unwrap(),
        ));
    }
    req
}

/// Malformed bodies and media types must reach the actual framework error boundary.
fn raw_request(
    method: &str,
    uri: &str,
    ip: Option<&str>,
    ct: Option<&str>,
    headers: &[(&str, String)],
    body: Vec<u8>,
) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(ct) = ct {
        request = request.header(header::CONTENT_TYPE, ct);
    }
    for (key, value) in headers {
        request = request.header(*key, value);
    }
    with_peer(request.body(Body::from(body)).unwrap(), ip)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_account_cancels_reserved_keeps_fulfilled(pool: PgPool) {
    let common::TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "u").await;
    let common::TestUser { id: me, token: tok } = common::user(&pool, "u").await;
    let wid: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1,'registry','active',$2,'L') RETURNING id")
        .bind(owner).bind(Uuid::new_v4().simple().to_string()[..10].to_string()).fetch_one(&pool).await.unwrap();
    let mut items = vec![];
    for (st, q) in [
        ("reserved", 2),
        ("reserved", 1),
        ("purchased", 2),
        ("delivered", 1),
    ] {
        let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1,'i',10,$2) RETURNING id").bind(wid).bind(q).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO claims (item_id, user_id, claimer_name, qty, status) VALUES ($1,$2,'me',$3,$4::text::claim_status)")
            .bind(it).bind(me).bind(q).bind(st).execute(&pool).await.unwrap();
        items.push(it);
    }
    let (s, _, _) = common::call(
        &pool,
        "DELETE",
        "/me",
        &[("cookie", format!("ws_session={tok}"))],
        Some(json!({"confirm":"DELETE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let sts: Vec<String> = sqlx::query_scalar("SELECT status::text FROM claims WHERE user_id=$1")
        .bind(me)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(sts.iter().filter(|s| *s == "cancelled").count(), 2);
    assert_eq!(
        sts.iter()
            .filter(|s| *s == "purchased" || *s == "delivered")
            .count(),
        2
    );
    let q: Vec<i32> =
        sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = ANY($1) ORDER BY id")
            .bind(&items)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        q,
        vec![0, 0, 2, 1],
        "reserved 回補，purchased/delivered 保留"
    );
    let aud: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='claim.account_delete_cancel' AND actor_id=$1").bind(me).fetch_one(&pool).await.unwrap();
    assert_eq!(aud, 2);
    let names: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM claims WHERE user_id=$1 AND claimer_name='已刪除的使用者'",
    )
    .bind(me)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(names, 4);
    common::assert_all_reconciled(&pool).await;
}

fn assert_problem(
    s: StatusCode,
    h: &axum::http::HeaderMap,
    b: &Value,
    want: StatusCode,
    code: &str,
) {
    assert_eq!(s, want);
    assert_eq!(h[header::CONTENT_TYPE], "application/problem+json");
    let v = b;
    assert_eq!(v["code"], code);
    assert_eq!(v["status"], want.as_u16());
    let t = b.to_string();
    assert!(
        !t.contains("LoginReq") && !t.contains("deserialize") && !t.contains("Cannot parse"),
        "洩漏內部資訊：{t}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn framework_errors_are_problem_json(pool: PgPool) {
    let login = "/api/v1/auth/login";
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request("POST", login, None, Some("text/plain"), &[], b"{}".to_vec()),
    )
    .await;
    assert_problem(
        s,
        &h,
        &b,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "UNSUPPORTED_MEDIA_TYPE",
    );
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "POST",
            login,
            None,
            Some("application/json"),
            &[],
            b"{oops".to_vec(),
        ),
    )
    .await;
    assert_problem(s, &h, &b, StatusCode::BAD_REQUEST, "BAD_REQUEST");
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "POST",
            login,
            None,
            Some("application/json"),
            &[],
            b"{}".to_vec(),
        ),
    )
    .await;
    assert_problem(
        s,
        &h,
        &b,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_FAILED",
    );
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "POST",
            login,
            None,
            Some("application/json"),
            &[],
            br#"{"email":"\ud800","password":"x"}"#.to_vec(),
        ),
    )
    .await;
    assert_problem(s, &h, &b, StatusCode::BAD_REQUEST, "BAD_REQUEST");
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "PATCH",
            "/api/v1/claims/not-a-uuid",
            None,
            Some("application/json"),
            &[],
            b"{}".to_vec(),
        ),
    )
    .await;
    assert_eq!(h[header::CONTENT_TYPE], "application/problem+json");
    assert!(
        s.is_client_error() && !b.to_string().contains("Cannot parse"),
        "{s} {b}"
    );
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "POST",
            login,
            None,
            Some("application/json"),
            &[],
            vec![b' '; 3 * 1024 * 1024],
        ),
    )
    .await;
    assert_problem(
        s,
        &h,
        &b,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PAYLOAD_TOO_LARGE",
    );
    // 自家 problem+json 不被改寫
    let (s, h, b) = common::call_request(
        &common::test_app(&pool),
        raw_request("GET", "/api/v1/me", None, None, &[], vec![]),
    )
    .await;
    assert_problem(s, &h, &b, StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn security_headers_on_all_responses(pool: PgPool) {
    for (m, u) in [
        ("GET", "/api/v1/me"),
        ("GET", "/api/v1/nope"),
        ("POST", "/api/v1/auth/login"),
    ] {
        let (_, h, _) = common::call_request(
            &common::test_app(&pool),
            raw_request(m, u, None, None, &[], vec![]),
        )
        .await;
        assert_eq!(h["x-content-type-options"], "nosniff", "{u}");
        assert_eq!(h["referrer-policy"], "no-referrer");
        assert_eq!(h["cache-control"], "no-store");
    }
    let common::TestUser { id: _, token: tok } = common::user(&pool, "u").await;
    let (_, h, _) = common::call(
        &pool,
        "GET",
        "/me/export",
        &[("cookie", format!("ws_session={tok}"))],
        None,
    )
    .await;
    assert_eq!(h["cache-control"], "private, no-store", "已設定者不覆蓋");
    assert_eq!(h["x-content-type-options"], "nosniff");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_lockout_is_per_email_ip_with_email_cap(pool: PgPool) {
    let email = format!("{}@example.com", Uuid::new_v4());
    let uid: Uuid =
        sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('u',$1) RETURNING id")
            .bind(&email)
            .fetch_one(&pool)
            .await
            .unwrap();
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let h = Argon2::default()
        .hash_password(b"correct-horse", &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, email, password_hash) VALUES ($1,'email',$2,$2,$3)").bind(uid).bind(&email).bind(h).execute(&pool).await.unwrap();
    let login = |ip: &'static str, pw: &'static str| {
        let (p, e) = (pool.clone(), email.clone());
        async move {
            common::call_request(
                &common::test_app(&p),
                with_peer(
                    common::request(
                        "POST",
                        &format!("/api/v1{}", "/auth/login"),
                        &[],
                        Some(json!({"email": e, "password": pw})),
                    ),
                    Some(ip),
                ),
            )
            .await
            .0
        }
    };
    for _ in 0..5 {
        assert_eq!(
            login("1.1.1.1", "bad-password").await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login("1.1.1.1", "correct-horse").await,
        StatusCode::TOO_MANY_REQUESTS,
        "攻擊者自己的 (email, IP) 被鎖"
    );
    assert_eq!(
        login("2.2.2.2", "correct-horse").await,
        StatusCode::OK,
        "受害者換 IP 不受影響"
    );
    // 分散式：5 個 IP 各 4 次（每組 < 5）= 20 → email 總量鎖
    for i in 0..5 {
        for _ in 0..4 {
            sqlx::query("INSERT INTO login_failures (email, ip) VALUES ($1,$2)")
                .bind(&email)
                .bind(format!("10.0.0.{i}"))
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    assert_eq!(
        login("3.3.3.3", "correct-horse").await,
        StatusCode::TOO_MANY_REQUESTS,
        "email 合計 20 次後鎖定"
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_19_distributed_failures_not_locked(pool: PgPool) {
    let email = "dist@example.com";
    for i in 0..19 {
        sqlx::query("INSERT INTO login_failures (email, ip) VALUES ($1,$2)")
            .bind(email)
            .bind(format!("10.1.0.{}", i % 5))
            .execute(&pool)
            .await
            .unwrap();
    }
    let bad = |ip| {
        let p = pool.clone();
        async move {
            common::call_request(
                &common::test_app(&p),
                with_peer(
                    common::request(
                        "POST",
                        &format!("/api/v1{}", "/auth/login"),
                        &[],
                        Some(json!({"email": email, "password": "bad-password"})),
                    ),
                    Some(ip),
                ),
            )
            .await
            .0
        }
    };
    assert_eq!(bad("4.4.4.4").await, StatusCode::UNAUTHORIZED); // 第 20 筆
    assert_eq!(bad("5.5.5.5").await, StatusCode::TOO_MANY_REQUESTS);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unsubscribe_post_allowed_in_read_only(pool: PgPool) {
    let common::TestUser { id: u, token: _ } = common::user(&pool, "u").await;
    sqlx::query("INSERT INTO system_flags (key, value) VALUES ('read_only','true') ON CONFLICT (key) DO UPDATE SET value='true'").execute(&pool).await.unwrap();
    let tok = wishly_api::account::unsub_token('u', u);
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/unsubscribe",
            &[],
            Some(json!({"token": tok}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/unsubscribe",
            &[],
            Some(json!({"token": "x"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn export_includes_contribution_visibility(pool: PgPool) {
    let common::TestUser { id: me, token: tok } = common::user(&pool, "u").await;
    let wid: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1, 'registry', 'active', $2, 'L') RETURNING id")
        .bind(me).bind(Uuid::new_v4().simple().to_string()[..10].to_string()).fetch_one(&pool).await.unwrap();
    let item: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, funding_mode, target_points, pledged_points, funding_status, funding_deadline, fulfillment_type)
        VALUES ($1, '推車', 1, 'crowdfund', 100, 10, 'open', now() + interval '7 days', 'concierge') RETURNING id")
        .bind(wid).fetch_one(&pool).await.unwrap();
    let wallet: Uuid =
        sqlx::query_scalar("INSERT INTO point_wallets (user_id) VALUES ($1) RETURNING id")
            .bind(me)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points, visibility) VALUES ($1, $2, $3, $4, 'u', 10, 'friends')")
        .bind(item).bind(wid).bind(me).bind(wallet).execute(&pool).await.unwrap();
    let (status, _, export) = common::call(
        &pool,
        "GET",
        "/me/export",
        &[("cookie", format!("ws_session={tok}"))],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(export["contributions"][0]["visibility"], "friends");
    common::assert_all_reconciled(&pool).await;
}

/// Tauri：不帶 cookie，以 Authorization: Bearer 驗證；X-Native-Client 登入才回 session_token；CORS 允許 tauri origin
#[sqlx::test(migrations = "../../db/migrations")]
async fn bearer_session_and_native_cors(pool: PgPool) {
    let common::TestUser { id: _, token: tok } = common::user(&pool, "u").await;
    let (s, _, _) = common::call(&pool, "GET", "/me", &[], None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, b) = common::call(
        &pool,
        "GET",
        "/me",
        &[("authorization", format!("Bearer {tok}"))],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _, _) = common::call(
        &pool,
        "GET",
        "/me",
        &[("authorization", "Bearer nope".into())],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // 登出以 Bearer 撤銷，之後同權杖失效
    let (s, _, _) = common::call(
        &pool,
        "POST",
        "/auth/logout",
        &[("authorization", format!("Bearer {tok}"))],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = common::call(
        &pool,
        "GET",
        "/me",
        &[("authorization", format!("Bearer {tok}"))],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // CORS preflight：tauri://localhost 通過、陌生來源不通過
    let (_, h, _) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "OPTIONS",
            "/api/v1/me",
            None,
            None,
            &[
                ("origin", "tauri://localhost".into()),
                ("access-control-request-method", "GET".into()),
                (
                    "access-control-request-headers",
                    "authorization,x-native-client".into(),
                ),
            ],
            vec![],
        ),
    )
    .await;
    assert_eq!(h["access-control-allow-origin"], "tauri://localhost");
    let (_, h, _) = common::call_request(
        &common::test_app(&pool),
        raw_request(
            "OPTIONS",
            "/api/v1/me",
            None,
            None,
            &[
                ("origin", "https://evil.example".into()),
                ("access-control-request-method", "GET".into()),
            ],
            vec![],
        ),
    )
    .await;
    assert!(h.get("access-control-allow-origin").is_none());
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn export_claims_include_is_anonymous(pool: PgPool) {
    let owner = common::user(&pool, "o").await;
    let me = common::user(&pool, "me").await;
    let wl = common::wishlist(&pool, owner.id, common::ListOpts::default()).await;
    let it = common::item(&pool, wl.id, "x", 1).await;
    let (st, _, c) = common::call(
        &pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[common::ck(&me.token), common::idem(Uuid::new_v4())].concat(),
        Some(serde_json::json!({"qty": 1, "is_anonymous": true})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    let (st, _, export) =
        common::call(&pool, "GET", "/me/export", &common::ck(&me.token), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(export["claims"][0]["is_anonymous"], true);
}

/// 成功登入只標記清除先前的失敗（後台分析仍算得到），鎖定 / 人機驗證門檻則重新計算；成功那次不算失敗。
#[sqlx::test(migrations = "../../db/migrations")]
async fn login_success_clears_lockout_but_keeps_failures_for_analytics(pool: PgPool) {
    let email = format!("{}@example.com", Uuid::new_v4());
    let uid: Uuid =
        sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('u',$1) RETURNING id")
            .bind(&email)
            .fetch_one(&pool)
            .await
            .unwrap();
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let h = Argon2::default()
        .hash_password(b"correct-horse", &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, email, password_hash) VALUES ($1,'email',$2,$2,$3)").bind(uid).bind(&email).bind(h).execute(&pool).await.unwrap();
    let login = |pw: &'static str| {
        let (p, e) = (pool.clone(), email.clone());
        async move {
            common::call_request(
                &common::test_app(&p),
                with_peer(
                    common::request(
                        "POST",
                        "/api/v1/auth/login",
                        &[],
                        Some(json!({"email": e, "password": pw})),
                    ),
                    Some("1.1.1.1"),
                ),
            )
            .await
            .0
        }
    };
    for _ in 0..4 {
        assert_eq!(login("bad-password").await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login("correct-horse").await, StatusCode::OK);
    // 已清除的 4 筆不再計入 (email, IP) 鎖定：再錯 2 次仍是 401（若照算，第 2 次會變 429）
    for _ in 0..2 {
        assert_eq!(
            login("bad-password").await,
            StatusCode::UNAUTHORIZED,
            "已清除的失敗不可再計入鎖定"
        );
    }
    let (total, open): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE cleared_at IS NULL) FROM login_failures WHERE email=$1",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (total, open),
        (6, 2),
        "成功那次不留紀錄，先前 4 筆保留但已清除"
    );
    common::assert_all_reconciled(&pool).await;
}
