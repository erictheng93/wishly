mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use common::{assert_all_reconciled, call_app as call, item, ListOpts, TestList};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::{claims, guest, public, AppState};

/// 建立 owner + 清單（active）；回傳 slug / id
async fn named_list(pool: &PgPool, slug: &str) -> TestList {
    let owner = common::user(pool, "小米").await;
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1")
        .bind(owner.id)
        .execute(pool)
        .await
        .unwrap();
    let mut list = common::wishlist(pool, owner.id, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET slug=$2,title='寶寶清單' WHERE id=$1")
        .bind(list.id)
        .bind(slug)
        .execute(pool)
        .await
        .unwrap();
    list.slug = slug.to_string();
    list
}

/// 只掛本切片的路由，不受其他模組影響
fn test_app(pool: &PgPool) -> Router {
    Router::new()
        .nest(
            "/api/v1",
            public::routes()
                .merge(guest::routes())
                .merge(claims::routes()),
        )
        .with_state(AppState { pool: pool.clone() })
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_happy_and_masking(pool: PgPool) {
    let app = test_app(&pool);
    let f = named_list(&pool, "Ab3dE5gH7k").await;
    sqlx::query("UPDATE wishlists SET show_claimer_names=true WHERE id=$1")
        .bind(f.id)
        .execute(&pool)
        .await
        .unwrap();
    let bottle = item(&pool, f.id, "玻璃奶瓶", 10).await;
    item(&pool, f.id, "推車", 2).await;
    let ming = common::user(&pool, "小明").await;
    let (st, _, c) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", bottle),
        &[common::ck(&ming.token), common::idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 5, "visibility": "public"})), // 會員預設層級非 public 時公開頁會匿名
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{c}");

    let (st, h, b) = call(&app, "GET", "/public/wishlists/Ab3dE5gH7k", &[], None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h["cache-control"]
        .to_str()
        .unwrap()
        .starts_with("public, s-maxage=10"));
    assert_eq!(b["title"], "寶寶清單");
    assert_eq!(b["claimers_visible"], true);
    assert_eq!(b["completion"]["completion_pct"], 42); // 5 / 12
    assert_eq!(b["items"][0]["qty_claimed"], 5);
    assert_eq!(b["items"][0]["qty_remaining"], 5);
    assert_eq!(
        b["items"][0]["claimers"][0],
        json!({"display_name": "小明", "qty": 5})
    );
    assert!(b.to_string().find("contact").is_none());

    // 匿名認領：即使 visibility=public，公開頁也只顯示「匿名」
    let hua = common::user(&pool, "小華").await;
    let (st, _, c) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", bottle),
        &[common::ck(&hua.token), common::idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 1, "visibility": "public", "is_anonymous": true})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    assert_eq!(c["claim"]["is_anonymous"], true);
    let (_, _, b) = call(&app, "GET", "/public/wishlists/Ab3dE5gH7k", &[], None).await;
    assert_eq!(
        b["items"][0]["claimers"],
        json!([{"display_name": "小明", "qty": 5}, {"display_name": "匿名", "qty": 1}])
    );
    assert!(!b.to_string().contains("小華"));

    // 驚喜鎖定（未來 event_date）→ 即使 show_claimer_names 也不輸出
    sqlx::query(
        "UPDATE wishlists SET surprise_mode = true, event_date = current_date + 30 WHERE id = $1",
    )
    .bind(f.id)
    .execute(&pool)
    .await
    .unwrap();
    let (_, _, b) = call(&app, "GET", "/public/wishlists/Ab3dE5gH7k", &[], None).await;
    assert_eq!(b["surprise_mode"], true);
    assert_eq!(b["claimers_visible"], false);
    assert!(b["items"][0].get("claimers").is_none());
    assert_eq!(b["items"][0]["qty_claimed"], 6);
    // 鎖定條件只有一份定義（0026）：沒有活動日一律視為鎖定
    let locked: bool = sqlx::query_scalar("SELECT surprise_locked(true, NULL)").fetch_one(&pool).await.unwrap();
    assert!(locked);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_404_and_410(pool: PgPool) {
    let app = test_app(&pool);
    let get_status = |slug: &'static str| {
        let app = app.clone();
        async move { call(&app, "GET", &format!("/public/wishlists/{slug}"), &[], None).await }
    };
    assert_eq!(get_status("Zz9999999z").await.0, StatusCode::NOT_FOUND);

    let deleted = named_list(&pool, "Deleted001").await;
    sqlx::query("UPDATE wishlists SET deleted_at = now() WHERE id = $1")
        .bind(deleted.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(get_status("Deleted001").await.0, StatusCode::NOT_FOUND);

    let draft = named_list(&pool, "Draft00001").await;
    sqlx::query("UPDATE wishlists SET status = 'draft' WHERE id = $1")
        .bind(draft.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(get_status("Draft00001").await.0, StatusCode::NOT_FOUND);

    let hidden = named_list(&pool, "Hidden0001").await;
    sqlx::query("UPDATE wishlists SET moderation_status='hidden',moderation_reason='詐騙',show_claimer_names=false WHERE id=$1")
        .bind(hidden.id).execute(&pool).await.unwrap();
    let (st, _, b) = get_status("Hidden0001").await;
    assert_eq!(
        (st, b["code"].as_str().unwrap()),
        (StatusCode::GONE, "WISHLIST_REMOVED")
    );
    assert!(!b.to_string().contains("詐騙"));
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn oversell_only_one_wins(pool: PgPool) {
    let app = test_app(&pool);
    let f = named_list(&pool, "Race000001").await;
    let it = item(&pool, f.id, "最後一件", 1).await;
    let mut users = vec![];
    for n in 0..8 {
        users.push(common::user(&pool, &format!("g{n}")).await);
    }
    let tasks: Vec<_> = users
        .into_iter()
        .map(|u| {
            let app = app.clone();
            tokio::spawn(async move {
                call(
                    &app,
                    "POST",
                    &format!("/items/{}/claims", it),
                    &[common::ck(&u.token), common::idem(Uuid::new_v4())].concat(),
                    Some(json!({"qty": 1})),
                )
                .await
            })
        })
        .collect();
    let mut ok = 0;
    for t in tasks {
        let (st, _, b) = t.await.unwrap();
        match st {
            StatusCode::CREATED => ok += 1,
            StatusCode::CONFLICT => {
                assert_eq!(b["code"], "ITEM_FULLY_CLAIMED");
                assert_eq!(b["remaining"], 0);
            }
            other => panic!("unexpected {other} {b}"),
        }
    }
    assert_eq!(ok, 1);
    let (claimed, n): (i32, i64) = sqlx::query_as("SELECT i.qty_claimed, (SELECT count(*) FROM claims WHERE item_id = i.id) FROM wishlist_items i WHERE i.id = $1")
        .bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!((claimed, n), (1, 1));
    // 新認捐不再建立 guest
    let guests: i64 = sqlx::query_scalar("SELECT count(*) FROM guests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(guests, 0);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotency_key_replay_and_conflict(pool: PgPool) {
    let app = test_app(&pool);
    let f = named_list(&pool, "Idem000001").await;
    let it = item(&pool, f.id, "奶瓶", 5).await;
    let key = Uuid::new_v4();
    let body = json!({"qty": 2});
    let ming = common::user(&pool, "小明").await;
    let hdr = |k: Uuid| [common::ck(&ming.token), common::idem(k)].concat();

    let (s1, h1, b1) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", it),
        &hdr(key),
        Some(body.clone()),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);
    assert!(h1.contains_key("location") && !h1.contains_key("set-cookie"));
    let (s2, h2, b2) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", it),
        &hdr(key),
        Some(body.clone()),
    )
    .await;
    assert_eq!(s2, StatusCode::CREATED);
    assert_eq!(h2["idempotency-replayed"], "true");
    assert_eq!(b1, b2);
    let (n, q): (i64, i32) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM claims), qty_claimed FROM wishlist_items WHERE id = $1",
    )
    .bind(it)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((n, q), (1, 2));

    let (s3, _, b3) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", it),
        &hdr(key),
        Some(json!({"qty": 3})),
    )
    .await;
    assert_eq!(
        (s3, b3["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT")
    );

    // 缺 key
    let r = Request::post(format!("/api/v1/items/{it}/claims"))
        .header("cookie", format!("ws_session={}", ming.token))
        .body(Body::from(body.to_string()))
        .unwrap();
    let (s4, _, b4) = common::call_request(&app, r).await;
    assert_eq!(
        (s4, b4["code"].as_str().unwrap()),
        (StatusCode::BAD_REQUEST, "IDEMPOTENCY_KEY_REQUIRED")
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_lifecycle(pool: PgPool) {
    let app = test_app(&pool);
    let f = named_list(&pool, "Life000001").await;
    let it = item(&pool, f.id, "奶瓶", 3).await;
    let ming = common::user(&pool, "小明").await;
    let tok = ming.token.clone();
    let (s, _, b) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", it),
        &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let cid = b["claim"]["id"].as_str().unwrap().to_string();

    // 重複捐助 → CLAIM_ALREADY_EXISTS（附 claim_id）
    let (s, _, e) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", it),
        &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 1})),
    )
    .await;
    assert_eq!(
        (
            s,
            e["code"].as_str().unwrap(),
            e["claim_id"].as_str().unwrap()
        ),
        (StatusCode::CONFLICT, "CLAIM_ALREADY_EXISTS", cid.as_str())
    );
    // 改量超過 → ITEM_FULLY_CLAIMED；合法改量
    let url = format!("/claims/{cid}");
    let (s, _, e) = call(
        &app,
        "PATCH",
        &url,
        &common::ck(&tok),
        Some(json!({"qty": 4})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "ITEM_FULLY_CLAIMED")
    );
    let (s, _, o) = call(
        &app,
        "PATCH",
        &url,
        &common::ck(&tok),
        Some(json!({"qty": 3, "status": "purchased"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!(
        (
            o["claim"]["status"].as_str().unwrap(),
            o["item"]["qty_claimed"].as_i64().unwrap()
        ),
        ("purchased", 3)
    );
    // 他人不可改；未登入 401
    let (s, _, _) = call(&app, "PATCH", &url, &[], Some(json!({"note": "x"}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // 取消回補，重複 DELETE 冪等
    for _ in 0..2 {
        let (s, _, _) = call(&app, "DELETE", &url, &common::ck(&tok), Some(Value::Null)).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
    }
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(q, 0);
    // 取消後可再捐助（partial unique 只算有效）
    let (s, _, _) = call(
        &app,
        "POST",
        &format!("/items/{}/claims", it),
        &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_all_reconciled(&pool).await;
}
