mod common;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::{claims, guest, ratelimit, reports, uploads, AppState};

fn app(pool: &PgPool) -> Router {
    Router::new()
        .nest(
            "/api/v1",
            claims::routes()
                .merge(guest::routes())
                .merge(reports::routes())
                .merge(uploads::routes()),
        )
        .with_state(AppState { pool: pool.clone() })
}

async fn call_with_ip(app: &Router, mut req: Request<Body>, ip: Option<&str>) -> common::Resp {
    if let Some(ip) = ip {
        req.extensions_mut()
            .insert(ConnectInfo::<std::net::SocketAddr>(
                format!("{ip}:1234").parse().unwrap(),
            ));
    }
    common::call_request(app, req).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn check_blocks_with_retry_after(pool: PgPool) {
    for _ in 0..2 {
        ratelimit::check(&pool, "k", 2, 3600).await.unwrap();
    }
    let e = ratelimit::check(&pool, "k", 2, 3600).await.unwrap_err();
    let res = axum::response::IntoResponse::into_response(e);
    assert_eq!(res.status(), 429);
    let ra: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=3600).contains(&ra));
    ratelimit::check(&pool, "other", 2, 3600).await.unwrap(); // key 獨立
    common::assert_all_reconciled(&pool).await;
}

fn report(slug: &str) -> Request<Body> {
    Request::post(format!("/api/v1/public/wishlists/{slug}/reports"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"reason": "scam"}).to_string()))
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn report_limits(pool: PgPool) {
    let a = app(&pool);
    for i in 0..11 {
        common::quantity_wishlist(&pool, &format!("RlRep{i:05}"), 0, 5).await;
    }
    // 同 IP 同清單 24h 1 筆
    assert_eq!(
        call_with_ip(&a, report("RlRep00000"), Some("2.2.2.2"))
            .await
            .0,
        StatusCode::CREATED
    );
    let (s, h, b) = call_with_ip(&a, report("RlRep00000"), Some("2.2.2.2")).await;
    assert_eq!(
        (s, b["code"].as_str().unwrap()),
        (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED")
    );
    assert!(h.contains_key("retry-after"));
    assert_eq!(
        call_with_ip(&a, report("RlRep00000"), Some("3.3.3.3"))
            .await
            .0,
        StatusCode::CREATED
    ); // 另一 IP 不受影響
       // 每 IP 每小時 10 筆（已用 1 筆於 2.2.2.2 的 report_ip；再 9 筆不同清單到第 10 筆，第 11 筆被擋）
       // 被擋的嘗試也計數：此 IP 已 2 次，再 8 筆到 10，下一筆被擋
    for i in 1..9 {
        assert_eq!(
            call_with_ip(&a, report(&format!("RlRep{i:05}")), Some("2.2.2.2"))
                .await
                .0,
            StatusCode::CREATED,
            "{i}"
        );
    }
    assert_eq!(
        call_with_ip(&a, report("RlRep00009"), Some("2.2.2.2"))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn report_known_reporter_duplicate_returns_same_id(pool: PgPool) {
    let a = app(&pool);
    let (wl, _) = common::quantity_wishlist(&pool, "RlRepDup01", 0, 5).await;
    let g: Uuid = sqlx::query_scalar(
        "INSERT INTO guests (guest_token_hash, display_name) VALUES ($1, 'g') RETURNING id",
    )
    .bind(wishly_api::session::hash_token("tok"))
    .fetch_one(&pool)
    .await
    .unwrap();
    let mk = || {
        let mut r = report("RlRepDup01");
        r.headers_mut()
            .insert("x-guest-token", "tok".parse().unwrap());
        r
    };
    let (_, _, b1) = call_with_ip(&a, mk(), Some("4.4.4.4")).await;
    let (s, _, b2) = call_with_ip(&a, mk(), Some("4.4.4.4")).await;
    assert_eq!((s, &b1["id"]), (StatusCode::CREATED, &b2["id"]));
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM content_reports WHERE wishlist_id = $1 AND reporter_guest_id = $2",
    )
    .bind(wl)
    .bind(g)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn presign_limit_per_user(pool: PgPool) {
    let a = app(&pool);
    let common::TestUser { id: uid, token } = common::user(&pool, "u").await;
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1")
        .bind(uid)
        .execute(&pool)
        .await
        .unwrap();
    // 預填：本小時已用 60 次 → 第 61 次 429（不必真的連 S3）
    sqlx::query("INSERT INTO rate_limits (key, window_start, count) VALUES ($1, to_timestamp(floor(extract(epoch FROM now()) / 3600) * 3600), 60)")
        .bind(format!("presign:{uid}")).execute(&pool).await.unwrap();
    let r = Request::post("/api/v1/uploads/presign")
        .header("content-type", "application/json")
        .header("cookie", format!("ws_session={token}"))
        .body(Body::from(
            json!({"purpose": "cover", "content_type": "image/png", "content_length": 1000})
                .to_string(),
        ))
        .unwrap();
    let (s, h, b) = call_with_ip(&a, r, None).await;
    assert_eq!(
        (s, b["code"].as_str().unwrap()),
        (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED")
    );
    assert!(h.contains_key("retry-after"));
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cleanup_removes_old_rows(pool: PgPool) {
    sqlx::query("INSERT INTO idempotency_keys (key, scope, request_hash, created_at) VALUES ($1, 's', 'h', now() - interval '25 hours'), ($2, 's', 'h', now())")
        .bind(Uuid::new_v4()).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO rate_limits (key, window_start, count) VALUES ('old', now() - interval '3 days', 1), ('new', now(), 1)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO login_failures (email, ip, created_at) SELECT 'a@x.com', '1.1.1.1', now() - make_interval(days => age) FROM unnest(ARRAY[366, 364, 2, 0]) age").execute(&pool).await.unwrap();
    let u = common::user(&pool, "u").await;
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) SELECT $1, decode(md5(age::text), 'hex'), now() - make_interval(days => age) FROM unnest(ARRAY[31, 29]) age")
        .bind(u.id).execute(&pool).await.unwrap();
    ratelimit::cleanup(&pool).await.unwrap();
    let (i, r, f, s): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM idempotency_keys), (SELECT count(*) FROM rate_limits), (SELECT count(*) FROM login_failures), (SELECT count(*) FROM sessions)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (i, r, f, s),
        (1, 1, 3, 2),
        "過期 31 天的 session 應清除，user() 的有效 session 與過期 29 天的保留"
    );
    common::assert_all_reconciled(&pool).await;
}

/// 並行的 cooldown 必須排隊：fixture 交易持同一把鎖並寫入未提交的通過紀錄，cooldown 要卡住，
/// 提交後才看得到那列而回 429；沒有鎖就會讀不到未提交列而直接放行（mutant otp-cooldown-lock）。
#[sqlx::test(migrations = "../../db/migrations")]
async fn cooldown_waits_on_key_lock_then_sees_committed_hit(pool: PgPool) {
    let mut fx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('cooldown:k', 0))")
        .execute(&mut *fx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO rate_limits (key, window_start, count) VALUES ('k', now(), 1)")
        .execute(&mut *fx)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *fx)
        .await
        .unwrap();
    let p = pool.clone();
    let h = tokio::spawn(async move { ratelimit::cooldown(&p, "k", 60).await });
    common::wait_for_blocked(&pool, pid, 1).await;
    fx.commit().await.unwrap();
    assert!(
        h.await.unwrap().is_err(),
        "cooldown 沒有等鎖，放行了並行請求"
    );
}

/// 回歸：固定視窗以 epoch 對齊，跨過 60 秒邊界的兩次請求會各算一次；cooldown 必須仍擋下。
#[sqlx::test(migrations = "../../db/migrations")]
async fn cooldown_holds_across_fixed_window_boundary(pool: PgPool) {
    // 上一次通過在「上一個 60 秒視窗」的最後一秒：距現在 < 60 秒，但固定視窗會當成不同視窗
    sqlx::query("INSERT INTO rate_limits (key, window_start, count) VALUES ('cd', to_timestamp(floor(extract(epoch FROM now()) / 60) * 60 - 1), 1)")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ratelimit::check_pool(&pool, "cd", 1, 60).await.is_ok(),
        "固定視窗在邊界另一側會放行"
    );
    assert!(ratelimit::cooldown(&pool, "cd", 60).await.is_err());
    // 通過後再要即被擋；超過間隔才再放行
    ratelimit::cooldown(&pool, "other", 60).await.unwrap();
    assert!(ratelimit::cooldown(&pool, "other", 60).await.is_err());
    sqlx::query(
        "UPDATE rate_limits SET window_start = now() - interval '61 seconds' WHERE key='other'",
    )
    .execute(&pool)
    .await
    .unwrap();
    ratelimit::cooldown(&pool, "other", 60).await.unwrap();
}
