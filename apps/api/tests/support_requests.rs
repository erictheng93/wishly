//! 客服工單（#92）：會員 / 未登入建立、驗證、上限、後台列表與回覆、通知、稽核、帳號刪除與保留期。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn req(category: &str, body: &str) -> Option<Value> {
    Some(json!({ "category": category, "body": body }))
}

async fn create(pool: &PgPool, tok: &str, body: Option<Value>) -> Resp {
    call(pool, "POST", "/me/support-requests", &ck(tok), body).await
}

async fn create_public(pool: &PgPool, ip: &str, body: Value) -> Resp {
    let mut r = request("POST", "/api/v1/public/support-requests", &[], Some(body));
    r.extensions_mut()
        .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
            format!("{ip}:1").parse().unwrap(),
        ));
    call_request(&test_app(pool), r).await
}

async fn set_roles(pool: &PgPool, id: Uuid, roles: &[&str]) {
    let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
    sqlx::query("UPDATE users SET staff_roles = $2 WHERE id = $1")
        .bind(id)
        .bind(&roles)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn member_create_validate_and_list_own(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    // 錢包凍結也能送出
    sqlx::query("INSERT INTO point_wallets(user_id,status) VALUES ($1,'frozen')")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, v) = create(&pool, &a.token, req("wallet", "  錢包被凍結了  ")).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(
        (v["status"].as_str(), v["body"].as_str()),
        (Some("open"), Some("錢包被凍結了"))
    );
    assert!(v.get("handled_by").is_none());

    // 未登入 401
    let (st, _, _) = call(
        &pool,
        "POST",
        "/me/support-requests",
        &[],
        req("other", "x"),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // 類別、空內文、過長、未知欄位
    for body in [
        req("nope", "x"),
        req("other", "   "),
        req("other", &"字".repeat(2001)),
        Some(json!({"category":"other","body":"x","extra":1})),
    ] {
        let (st, _, v) = create(&pool, &a.token, body).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    }
    // 他人的清單 / 儲值單 → 422；自己的可以
    let other_wl = wishlist(&pool, b.id, ListOpts::default()).await;
    let own_wl = wishlist(&pool, a.id, ListOpts::default()).await;
    let other_tp = topup_credit(&pool, b.id, 100).await;
    for body in [
        json!({"category":"wishlist","body":"x","wishlist_id": other_wl.id}),
        json!({"category":"wishlist","body":"x","wishlist_id": Uuid::new_v4()}),
        json!({"category":"payment","body":"x","topup_id": other_tp}),
    ] {
        let (st, _, v) = create(&pool, &a.token, Some(body)).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    }
    let own_tp = topup_credit(&pool, a.id, 100).await;
    let (st, _, v) = create(
        &pool,
        &a.token,
        Some(json!({"category":"wishlist","body":"清單問題","wishlist_id": own_wl.id, "topup_id": own_tp})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");

    // 只看得到自己的，新到舊
    create(&pool, &b.token, req("other", "b 的")).await;
    let (st, _, v) = call(&pool, "GET", "/me/support-requests", &ck(&a.token), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = v["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["body"], "清單問題");
    assert!(rows.iter().all(|r| r["body"] != "b 的"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn member_open_limit(pool: PgPool) {
    let a = user(&pool, "a").await;
    for i in 0..5 {
        let (st, _, _) = create(&pool, &a.token, req("other", &format!("#{i}"))).await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let (st, _, v) = create(&pool, &a.token, req("other", "第六筆")).await;
    assert_eq!(
        (st, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("SUPPORT_OPEN_LIMIT"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_list_reply_roles_notify_audit(pool: PgPool) {
    let a = user(&pool, "a").await;
    let admin = staff(&pool, "admin").await;
    let fin = staff(&pool, "fin").await;
    let sup = staff(&pool, "sup").await;
    set_roles(&pool, fin.id, &["finance"]).await;
    set_roles(&pool, sup.id, &["support"]).await;
    let (_, _, t1) = create(&pool, &a.token, req("account", "會員的祕密內文")).await;
    let (_, _, t2) = create(&pool, &a.token, req("other", "第二筆")).await;
    let (st, _, _) = create_public(
        &pool,
        "10.1.0.1",
        json!({"email":"Guest@Example.com","category":"account","body":"被停權了"}),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // 一般使用者不能看後台；finance 可讀
    let (st, _, _) = call(&pool, "GET", "/admin/support-requests", &ck(&a.token), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, v) = call(
        &pool,
        "GET",
        "/admin/support-requests?status=open",
        &ck(&fin.token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let rows = v["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let member = rows.iter().find(|r| r["id"] == t1["id"]).unwrap();
    assert_eq!(member["kind"], "member");
    assert_eq!(member["user"]["id"], json!(a.id));
    assert!(member["user"].get("email").is_none() && member["contact_email"].is_null());
    let guest = rows.iter().find(|r| r["kind"] == "guest").unwrap();
    assert_eq!(guest["contact_email"], "guest@example.com");
    assert!(guest["user"].is_null());
    let (st, _, _) = call(
        &pool,
        "GET",
        "/admin/support-requests?status=bad",
        &ck(&fin.token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);

    // 後台待辦計數：會員與未登入合計
    let (_, _, s) = call(&pool, "GET", "/admin/stats", &ck(&fin.token), None).await;
    assert_eq!(s["open_support_requests"], 3);

    // finance 不能回覆；support 與 admin 可以
    let path = |v: &Value| format!("/admin/support-requests/{}", v["id"].as_str().unwrap());
    let body = Some(json!({"reply":"已協助處理"}));
    let (st, _, _) = call(&pool, "PATCH", &path(&t1), &ck(&fin.token), body.clone()).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = call(
        &pool,
        "PATCH",
        &path(&t1),
        &ck(&sup.token),
        Some(json!({"reply":"  "})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let (st, _, v) = call(&pool, "PATCH", &path(&t1), &ck(&sup.token), body.clone()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (v["status"].as_str(), v["reply"].as_str()),
        (Some("closed"), Some("已協助處理"))
    );
    let (st, _, v) = call(&pool, "PATCH", &path(&t1), &ck(&admin.token), body.clone()).await;
    assert_eq!(
        (st, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );
    let (st, _, _) = call(&pool, "PATCH", &path(&t2), &ck(&admin.token), body.clone()).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(
        &pool,
        "PATCH",
        &format!("/admin/support-requests/{}", Uuid::new_v4()),
        &ck(&admin.token),
        body.clone(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = call(&pool, "PATCH", &path(guest), &ck(&sup.token), body.clone()).await;
    assert_eq!(st, StatusCode::OK);

    // 使用者看得到回覆、看不到 handled_by
    let (_, _, v) = call(&pool, "GET", "/me/support-requests", &ck(&a.token), None).await;
    let mine = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == t1["id"])
        .unwrap()
        .clone();
    assert_eq!(
        (mine["status"].as_str(), mine["reply"].as_str()),
        (Some("closed"), Some("已協助處理"))
    );
    assert!(mine["handled_at"].is_string() && mine.get("handled_by").is_none());

    // 會員工單各一則 support.replied（payload 不含回覆），匿名工單不發
    let (n, all): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE user_id = $1 AND payload::text NOT LIKE '%已協助%'), count(*) FROM notifications WHERE kind = 'support.replied'")
        .bind(a.id).fetch_one(&pool).await.unwrap();
    assert_eq!((n, all), (2, 2));

    // 稽核 diff 不含內文、回覆、Email
    let diffs: Vec<String> = sqlx::query_scalar(
        "SELECT diff::text FROM audit_logs WHERE action = 'support_request.reply'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(diffs.len(), 3);
    assert!(
        diffs.iter().all(|d| !d.contains("祕密")
            && !d.contains("已協助")
            && !d.contains("@")
            && d.contains("closed")),
        "{diffs:?}"
    );

    let (_, _, v) = call(
        &pool,
        "GET",
        "/admin/support-requests?status=closed",
        &ck(&fin.token),
        None,
    )
    .await;
    assert_eq!(v["data"].as_array().unwrap().len(), 3);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_create_rules(pool: PgPool) {
    let existing = user(&pool, "existing").await;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(existing.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let ok = |e: &str| json!({"email": e, "category": "account", "body": "我被停權了"});

    let (st, _, v1) = create_public(&pool, "10.2.0.1", ok("nobody@example.com")).await;
    assert_eq!(st, StatusCode::CREATED);
    // 已註冊的 Email：回應形狀相同、不掛 user_id
    let (st, _, v2) = create_public(&pool, "10.2.0.2", ok(&email)).await;
    assert_eq!(st, StatusCode::CREATED);
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&v1), keys(&v2));
    let (uid, ce): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT user_id, contact_email FROM support_requests WHERE id = $1")
            .bind(Uuid::parse_str(v2["id"].as_str().unwrap()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((uid, ce), (None, Some(email.clone())));
    // 已登入的 cookie 也不會被讀
    let mut r = request(
        "POST",
        "/api/v1/public/support-requests",
        &ck(&existing.token),
        Some(ok("c@example.com")),
    );
    r.extensions_mut()
        .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
            "10.2.0.3:1".parse().unwrap(),
        ));
    let (st, _, v) = call_request(&test_app(&pool), r).await;
    assert_eq!(st, StatusCode::CREATED);
    let uid: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM support_requests WHERE id = $1")
            .bind(Uuid::parse_str(v["id"].as_str().unwrap()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(uid, None);

    // Email 格式錯、帶 wishlist_id（未知欄位）→ 422
    let (st, _, _) = create_public(&pool, "10.2.0.4", ok("not-an-email")).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let mut b = ok("x@example.com");
    b["wishlist_id"] = json!(Uuid::new_v4());
    let (st, _, _) = create_public(&pool, "10.2.0.4", b).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);

    // 同 Email 同時 open ≤ 3（不分大小寫）
    for i in 0..2 {
        let (st, _, _) =
            create_public(&pool, &format!("10.3.0.{i}"), ok("NOBODY@example.com")).await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let (st, _, v) = create_public(&pool, "10.3.0.9", ok("nobody@example.com")).await;
    assert_eq!(
        (st, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("SUPPORT_OPEN_LIMIT"))
    );

    // 每 IP 限流
    for i in 0..30 {
        let (st, _, _) = create_public(&pool, "10.4.0.1", ok(&format!("ip{i}@example.com"))).await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let (st, _, _) = create_public(&pool, "10.4.0.1", ok("ip9@example.com")).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn check_constraint_deletion_and_retention(pool: PgPool) {
    let a = user(&pool, "a").await;
    for (uid, email) in [(None, None), (Some(a.id), Some("x@example.com"))] {
        let r = sqlx::query("INSERT INTO support_requests (user_id, contact_email, category, body) VALUES ($1, $2, 'other', 'x')")
            .bind(uid).bind(email).execute(&pool).await;
        assert!(r.is_err(), "{uid:?} {email:?}");
    }

    // 保留期：closed 且處理超過 365 天者刪除；open 與較新的保留
    let ins = |status: &'static str, days: i32| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>("INSERT INTO support_requests (contact_email, category, body, status, handled_at)
                VALUES ('r@example.com', 'other', 'x', $1, CASE WHEN $1 = 'closed' THEN now() - make_interval(days => $2) END) RETURNING id")
                .bind(status).bind(days).fetch_one(&pool).await.unwrap()
        }
    };
    let old = ins("closed", 400).await;
    let recent = ins("closed", 30).await;
    let open = ins("open", 0).await;
    sqlx::query(
        "UPDATE support_requests SET created_at = now() - interval '800 days' WHERE id = $1",
    )
    .bind(open)
    .execute(&pool)
    .await
    .unwrap();
    wishly_api::ratelimit::cleanup(&pool).await.unwrap();
    let left: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM support_requests WHERE contact_email = 'r@example.com'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!left.contains(&old) && left.contains(&recent) && left.contains(&open));

    // 帳號刪除清掉該使用者的工單
    create(&pool, &a.token, req("account", "刪帳號前")).await;
    let (st, _, _) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&a.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM support_requests WHERE user_id = $1")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
