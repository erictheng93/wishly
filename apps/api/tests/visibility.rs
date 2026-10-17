//! 清單可見性：6 種 visibility × 擁有者/匿名/好友/非好友/名單/密碼；捐助門檻、unlock、快取標頭、allowed-users
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishly_api::{app, AppState};

use common::*;

struct Ctx {
    owner: TestUser,
    friend: TestUser,
    stranger: TestUser,
    allowed: TestUser,
}
async fn ctx(pool: &PgPool) -> Ctx {
    let c = Ctx {
        owner: user(pool, "主人").await,
        friend: user(pool, "好友").await,
        stranger: user(pool, "路人").await,
        allowed: user(pool, "名單").await,
    };
    befriend(pool, c.owner.id, c.friend.id).await;
    befriend(pool, c.owner.id, c.allowed.id).await;
    c
}

/// 經 API 建立 active 清單（含一個品項）；回 (id, slug, item_id)
async fn mk(pool: &PgPool, c: &Ctx, vis: &str, pw: Option<&str>) -> (Uuid, String, Uuid) {
    let mut b = json!({ "type": "personal", "title": "清單", "visibility": vis });
    if let Some(p) = pw {
        b["access_password"] = json!(p);
    }
    let (s, _, w) = call(pool, "POST", "/wishlists", &ck(&c.owner.token), Some(b)).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let id: Uuid = w["id"].as_str().unwrap().parse().unwrap();
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(id).fetch_one(pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET status='active' WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    if vis == "selected" {
        let (s, _, v) = call(
            pool,
            "PUT",
            &format!("/wishlists/{id}/allowed-users"),
            &ck(&c.owner.token),
            Some(json!({ "user_ids": [c.allowed.id] })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    (id, w["slug"].as_str().unwrap().into(), it)
}

/// 受限內容路徑（依 cookie / X-List-Access 判斷）
async fn get(
    pool: &PgPool,
    slug: &str,
    hdr: &[(&str, String)],
) -> (StatusCode, axum::http::HeaderMap, Value) {
    call(
        pool,
        "GET",
        &format!("/restricted/wishlists/{slug}"),
        hdr,
        None,
    )
    .await
}

/// 可被 CDN 快取的公開路徑
async fn get_pub(
    pool: &PgPool,
    slug: &str,
    hdr: &[(&str, String)],
) -> (StatusCode, axum::http::HeaderMap, Value) {
    call(pool, "GET", &format!("/public/wishlists/{slug}"), hdr, None).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_and_link_open_and_cacheable(pool: PgPool) {
    let c = ctx(&pool).await;
    for vis in ["public", "link"] {
        let (_, slug, _) = mk(&pool, &c, vis, None).await;
        let (s, h, v) = get_pub(&pool, &slug, &[]).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["visibility"], vis);
        assert!(h["cache-control"].to_str().unwrap().starts_with("public"));
        assert!(h.get("cloudflare-cdn-cache-control").is_none());
        let etag = h["etag"].to_str().unwrap().to_string();
        assert_eq!(
            get_pub(&pool, &slug, &[("if-none-match", etag)]).await.0,
            StatusCode::NOT_MODIFIED
        );
        // 受限路徑也能讀，但永遠 no-store
        let (s, h, _) = get(&pool, &slug, &[]).await;
        assert_eq!(
            (s, h["cache-control"].to_str().unwrap()),
            (StatusCode::OK, "private, no-store")
        );
    }
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn private_only_owner(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "private", None).await;
    assert_eq!(get(&pool, &slug, &[]).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        get(&pool, &slug, &ck(&c.friend.token)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&pool, &slug, &ck(&c.owner.token)).await.0,
        StatusCode::OK
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friends_list(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "friends", None).await;
    let (s, _, v) = get(&pool, &slug, &[]).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("LOGIN_REQUIRED"))
    );
    assert_eq!(v["visibility"], "friends");
    assert_eq!(v["owner"]["display_name"], "主人");
    let (s, _, v) = get(&pool, &slug, &ck(&c.stranger.token)).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("FRIENDS_ONLY"))
    );
    for t in [&c.friend.token, &c.owner.token] {
        let (s, h, v) = get(&pool, &slug, &ck(t)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["visibility"], "friends");
        assert_eq!(h["cache-control"], "private, no-store");
        assert!(h.get("etag").is_none());
    }
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn selected_list(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "selected", None).await;
    assert_eq!(get(&pool, &slug, &[]).await.2["code"], "LOGIN_REQUIRED");
    // 好友但不在名單 / 路人 → NOT_ALLOWED
    for t in [&c.friend.token, &c.stranger.token] {
        let (s, _, v) = get(&pool, &slug, &ck(t)).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("NOT_ALLOWED"))
        );
    }
    let (s, h, _) = get(&pool, &slug, &ck(&c.allowed.token)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(
        get(&pool, &slug, &ck(&c.owner.token)).await.0,
        StatusCode::OK
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn password_list_unlock_and_tokens(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let (s, _, v) = get(&pool, &slug, &[]).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("PASSWORD_REQUIRED"))
    );
    assert_eq!(
        get(&pool, &slug, &[("x-list-access", "bogus".into())])
            .await
            .2["code"],
        "PASSWORD_REQUIRED"
    );
    // 錯密碼 / 缺密碼
    let u = format!("/public/wishlists/{slug}/unlock");
    assert_eq!(
        call(&pool, "POST", &u, &[], Some(json!({}))).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    ); // 缺 password 欄位
    let (s, _, v) = call(
        &pool,
        "POST",
        &u,
        &[],
        Some(json!({ "password": "wrong-password" })),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("WRONG_PASSWORD"))
    );
    let (s, _, v) = call(
        &pool,
        "POST",
        &u,
        &[],
        Some(json!({ "password": "correct-horse" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let tok = v["access_token"].as_str().unwrap().to_string();
    let (s, h, v) = get(&pool, &slug, &[("x-list-access", tok.clone())]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (
            v["visibility"].as_str(),
            h["cache-control"].to_str().unwrap()
        ),
        (Some("password"), "private, no-store")
    );
    assert!(h.get("etag").is_none());
    // 擁有者免密碼
    assert_eq!(
        get(&pool, &slug, &ck(&c.owner.token)).await.0,
        StatusCode::OK
    );
    // SSE 與檢舉走同一道門
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/public/wishlists/{slug}/events"),
            &[],
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // 通過時是串流，不能 collect body，只看狀態碼
    let req = Request::builder()
        .uri(format!(
            "/api/v1/public/wishlists/{slug}/events?access={tok}"
        ))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app(AppState { pool: pool.clone() })
            .oneshot(req)
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let rep = json!({ "reason": "other" });
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/public/wishlists/{slug}/reports"),
            &[],
            Some(rep.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/public/wishlists/{slug}/reports"),
            &[("x-list-access", tok.clone())],
            Some(rep)
        )
        .await
        .0,
        StatusCode::CREATED
    );
    // 換密碼 → 舊權杖失效
    let id: Uuid = sqlx::query_scalar("SELECT id FROM wishlists WHERE slug=$1")
        .bind(&slug)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &ck(&c.owner.token),
        Some(json!({ "access_password": "another-pass1" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        get(&pool, &slug, &[("x-list-access", tok)]).await.0,
        StatusCode::FORBIDDEN
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unlock_non_password_404_and_rate_limited(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, link, _) = mk(&pool, &c, "link", None).await;
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/public/wishlists/{link}/unlock"),
            &[],
            Some(json!({ "password": "whatever-123" }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, slug, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let u = format!("/public/wishlists/{slug}/unlock");
    assert_eq!(
        call(
            &pool,
            "POST",
            &u,
            &[],
            Some(json!({ "password": "wrong-password" }))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 直接把計數推到上限（不跑 10 次 argon2）：固定視窗對齊時鐘，慢的迴圈若跨過視窗邊界計數會歸零而偶發失敗
    sqlx::query("UPDATE rate_limits SET count = 10 WHERE key LIKE $1")
        .bind(format!("unlock:{slug}:%"))
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, v) = call(
        &pool,
        "POST",
        &u,
        &[],
        Some(json!({ "password": "correct-horse" })),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED"))
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_requires_access(pool: PgPool) {
    let c = ctx(&pool).await;
    let claim = |it: Uuid, mut h: Vec<(&'static str, String)>| {
        let pool = pool.clone();
        async move {
            h.push(("idempotency-key", Uuid::new_v4().to_string()));
            call(
                &pool,
                "POST",
                &format!("/items/{it}/claims"),
                &h,
                Some(json!({ "qty": 1 })),
            )
            .await
        }
    };
    let (_, _, it) = mk(&pool, &c, "friends", None).await;
    // 認捐一律需登入：未登入 401
    let (s, _, v) = claim(it, vec![]).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED"))
    );
    assert_eq!(
        claim(it, ck(&c.stranger.token)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(claim(it, ck(&c.friend.token)).await.0, StatusCode::CREATED);

    let (_, _, it) = mk(&pool, &c, "selected", None).await;
    assert_eq!(
        claim(it, ck(&c.friend.token)).await.2["code"],
        "NOT_ALLOWED"
    );
    assert_eq!(claim(it, ck(&c.allowed.token)).await.0, StatusCode::CREATED);

    let (_, slug, it) = mk(&pool, &c, "password", Some("correct-horse")).await;
    assert_eq!(claim(it, vec![]).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        claim(it, ck(&c.stranger.token)).await.2["code"],
        "PASSWORD_REQUIRED"
    );
    let (_, _, v) = call(
        &pool,
        "POST",
        &format!("/public/wishlists/{slug}/unlock"),
        &[],
        Some(json!({ "password": "correct-horse" })),
    )
    .await;
    let tok = v["access_token"].as_str().unwrap().to_string();
    assert_eq!(
        claim(
            it,
            [ck(&c.stranger.token), vec![("x-list-access", tok)]].concat()
        )
        .await
        .0,
        StatusCode::CREATED
    );

    let (_, _, it) = mk(&pool, &c, "private", None).await;
    assert_eq!(
        claim(it, ck(&c.friend.token)).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, _, it) = mk(&pool, &c, "public", None).await;
    assert_eq!(claim(it, vec![]).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        claim(it, ck(&c.stranger.token)).await.0,
        StatusCode::CREATED
    );
    assert_all_reconciled(&pool).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), pool.close())
        .await
        .expect("SSE listener must release its connection when the pool closes");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_password_rules_and_no_hash_leak(pool: PgPool) {
    let c = ctx(&pool).await;
    let o = ck(&c.owner.token);
    let bad = |b: Value| {
        let (p, o) = (pool.clone(), o.clone());
        async move { call(&p, "POST", "/wishlists", &o, Some(b)).await }
    };
    assert_eq!(
        bad(json!({ "type": "personal", "title": "x", "visibility": "password" }))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(bad(json!({ "type": "personal", "title": "x", "visibility": "password", "access_password": "short" })).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        bad(json!({ "type": "personal", "title": "x", "visibility": "nope" }))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let (id, _, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let (_, _, w) = call(&pool, "GET", &format!("/wishlists/{id}"), &o, None).await;
    assert_eq!(w["wishlist"]["has_password"], true);
    assert!(!w.to_string().contains("argon2"));
    // 離開 password → 清空雜湊；再進入須重新給密碼
    let (s, _, w) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &o,
        Some(json!({ "visibility": "link" })),
    )
    .await;
    assert_eq!((s, &w["has_password"]), (StatusCode::OK, &json!(false)));
    let h: Option<String> =
        sqlx::query_scalar("SELECT access_password_hash FROM wishlists WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(h.is_none());
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{id}"),
            &o,
            Some(json!({ "visibility": "password" }))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, w) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &o,
        Some(json!({ "visibility": "password", "access_password": "new-password-1" })),
    )
    .await;
    assert_eq!((s, &w["has_password"]), (StatusCode::OK, &json!(true)));
    // 已有密碼時改其他欄位不需重給
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{id}"),
            &o,
            Some(json!({ "title": "新標題" }))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn allowed_users_only_friends(pool: PgPool) {
    let c = ctx(&pool).await;
    let (id, _, _) = mk(&pool, &c, "selected", None).await;
    let u = format!("/wishlists/{id}/allowed-users");
    let (s, _, v) = call(
        &pool,
        "PUT",
        &u,
        &ck(&c.owner.token),
        Some(json!({ "user_ids": [c.friend.id, c.stranger.id] })),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("NOT_FRIEND"))
    );
    // 失敗不改動既有名單
    let (_, _, v) = call(&pool, "GET", &u, &ck(&c.owner.token), None).await;
    assert_eq!(v["users"].as_array().unwrap().len(), 1);
    let (s, _, v) = call(
        &pool,
        "PUT",
        &u,
        &ck(&c.owner.token),
        Some(json!({ "user_ids": [c.friend.id, c.allowed.id, c.friend.id] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["users"].as_array().unwrap().len(), 2);
    assert_eq!(
        call(
            &pool,
            "PUT",
            &u,
            &ck(&c.owner.token),
            Some(json!({ "user_ids": [] }))
        )
        .await
        .2["users"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // 非擁有者 → 404
    assert_eq!(
        call(&pool, "GET", &u, &ck(&c.friend.token), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "PUT",
            &u,
            &ck(&c.friend.token),
            Some(json!({ "user_ids": [] }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_all_reconciled(&pool).await;
}

/// CDN 防線：公開（可快取）路徑不論帶什麼憑證，都不會回受限清單內容；擋下的回應帶 CDN 專用 no-store。
#[sqlx::test(migrations = "../../db/migrations")]
async fn public_path_never_serves_restricted_content(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, pw_slug, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let v = call(
        &pool,
        "POST",
        &format!("/public/wishlists/{pw_slug}/unlock"),
        &[],
        Some(json!({ "password": "correct-horse" })),
    )
    .await
    .2;
    let tok = v["access_token"].as_str().unwrap().to_string();
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("friends", ck(&c.friend.token)),
        ("friends", ck(&c.owner.token)),
        ("selected", ck(&c.allowed.token)),
        ("private", ck(&c.owner.token)),
    ];
    for (vis, hdr) in cases {
        let (_, slug, _) = mk(&pool, &c, vis, None).await;
        assert_eq!(
            get(&pool, &slug, &hdr).await.0,
            StatusCode::OK,
            "{vis} 受限路徑可讀"
        );
        let (s, h, v) = get_pub(&pool, &slug, &hdr).await;
        assert!(
            matches!(s, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{vis}: {s}"
        );
        assert!(
            v.get("items").is_none() && v.get("slug").is_none(),
            "{vis} 公開路徑不得含清單內容：{v}"
        );
        assert_eq!(
            (
                h["cloudflare-cdn-cache-control"].to_str().unwrap(),
                h["cdn-cache-control"].to_str().unwrap()
            ),
            ("no-store", "no-store")
        );
    }
    let (s, h, v) = get_pub(&pool, &pw_slug, &[("x-list-access", tok.clone())]).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("PASSWORD_REQUIRED"))
    );
    assert_eq!(h["cloudflare-cdn-cache-control"], "no-store");
    assert_eq!(
        get(&pool, &pw_slug, &[("x-list-access", tok)]).await.0,
        StatusCode::OK
    );
    assert_all_reconciled(&pool).await;
}
