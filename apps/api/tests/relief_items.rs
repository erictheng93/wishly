//! 據點品項：category / urgency 驗證、數量認領、眾籌寄往據點地址、暫停 / 結束擋新認捐。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

struct Fx {
    owner: TestUser,
    site: TestList,
}
async fn fx(pool: &PgPool) -> Fx {
    let owner = user(pool, "O").await;
    let o = org(pool, owner.id, "verified").await;
    member(pool, o, owner.id, "owner").await;
    let p = permit(pool, o, "approved", 30).await;
    let (e, _) = event(pool, o).await;
    let site = site(pool, o, owner.id, e, p, "active").await;
    Fx { owner, site }
}
async fn add(pool: &PgPool, f: &Fx, body: Value) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "POST",
        &format!("/wishlists/{}/items", f.site.id),
        &ck(&f.owner.token),
        Some(body),
    )
    .await;
    (s, v)
}
fn pointer(v: &Value) -> (&str, &str) {
    (
        v["errors"][0]["pointer"].as_str().unwrap(),
        v["errors"][0]["code"].as_str().unwrap(),
    )
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_item_requires_category_urgency_422(pool: PgPool) {
    let f = fx(&pool).await;
    let (s, v) = add(
        &pool,
        &f,
        json!({"title":"水","qty_needed":5,"urgency":"high"}),
    )
    .await;
    assert_eq!(
        (s, pointer(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, ("/category", "REQUIRED"))
    );
    let (s, v) = add(
        &pool,
        &f,
        json!({"title":"水","qty_needed":5,"category":"water"}),
    )
    .await;
    assert_eq!(
        (s, pointer(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, ("/urgency", "REQUIRED"))
    );
    let (s, v) = add(
        &pool,
        &f,
        json!({"title":"水","qty_needed":5,"category":"gold","urgency":"high"}),
    )
    .await;
    assert_eq!(
        (s, pointer(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, ("/category", "ENUM"))
    );
    let (s, v) = add(
        &pool,
        &f,
        json!({"title":"水","qty_needed":5,"category":"water","urgency":"high"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(
        (
            v["category"].as_str(),
            v["urgency"].as_str(),
            v["qty_received"].as_i64()
        ),
        (Some("water"), Some("high"), Some(0))
    );
    // PATCH 可改、不可改成非法值
    let url = format!("/items/{}", v["id"].as_str().unwrap());
    let (s, _, u) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&f.owner.token),
        Some(json!({"urgency":"critical"})),
    )
    .await;
    assert_eq!(
        (s, u["urgency"].as_str(), u["category"].as_str()),
        (StatusCode::OK, Some("critical"), Some("water"))
    );
    let (s, _, u) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&f.owner.token),
        Some(json!({"urgency":"x"})),
    )
    .await;
    assert_eq!(
        (s, pointer(&u)),
        (StatusCode::UNPROCESSABLE_ENTITY, ("/urgency", "ENUM"))
    );
    let (s, _, u) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&f.owner.token),
        Some(json!({"category":null})),
    )
    .await;
    assert_eq!(
        (s, pointer(&u)),
        (StatusCode::UNPROCESSABLE_ENTITY, ("/category", "REQUIRED"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn non_relief_item_rejects_category_422(pool: PgPool) {
    let o = user(&pool, "個人").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/items", wl.id),
        &ck(&o.token),
        Some(json!({"title":"書","category":"water"})),
    )
    .await;
    assert_eq!(
        (s, pointer(&v)),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            ("/category", "NOT_APPLICABLE")
        )
    );
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/items", wl.id),
        &ck(&o.token),
        Some(json!({"title":"書"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

async fn relief_item(pool: &PgPool, wl: Uuid, cf: bool) -> Uuid {
    let q = if cf {
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,target_points,funding_status,funding_deadline,fulfillment_type,category,urgency)
         VALUES($1,'發電機',1,'crowdfund',100,'open',now()+interval '7 days','concierge','other','high') RETURNING id"
    } else {
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'礦泉水',10,'water','high') RETURNING id"
    };
    sqlx::query_scalar(q)
        .bind(wl)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn claim(pool: &PgPool, item: Uuid, tok: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "POST",
        &format!("/items/{item}/claims"),
        &[ck(tok), idem(Uuid::new_v4())].concat(),
        Some(json!({"qty":1})),
    )
    .await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_quantity_claim_works(pool: PgPool) {
    let f = fx(&pool).await;
    sqlx::query("UPDATE wishlists SET claim_ttl_hours=48 WHERE id=$1")
        .bind(f.site.id)
        .execute(&pool)
        .await
        .unwrap(); // 同 POST /orgs/{id}/sites 的預設
    let item = relief_item(&pool, f.site.id, false).await;
    let u = user(&pool, "捐").await;
    let (s, v) = claim(&pool, item, &u.token).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let secs: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - now())::float8 FROM claims WHERE id=$1",
    )
    .bind(v["claim"]["id"].as_str().unwrap().parse::<Uuid>().unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((secs - 48.0 * 3600.0).abs() < 300.0, "{secs}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_crowdfund_funds_and_orders_to_site_address(pool: PgPool) {
    let f = fx(&pool).await;
    let sealed = |s: &str| wishly_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO shipping_addresses(wishlist_id,recipient_name_enc,phone_enc,address_enc) VALUES($1,$2,$3,$4)")
        .bind(f.site.id).bind(sealed("王小明")).bind(sealed("0912-345-678")).bind(sealed("花蓮縣某處"))
        .execute(&pool).await.unwrap();
    let item = relief_item(&pool, f.site.id, true).await;
    let u = user(&pool, "贊").await;
    grant(&pool, u.id, 100).await;
    let app = test_app(&pool);
    let (s, _, v) = pledge(&app, item, &u.token, 100).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let snap: Vec<u8> = sqlx::query_scalar(
        "SELECT shipping_address_snapshot FROM purchase_orders WHERE item_id=$1",
    )
    .bind(item)
    .fetch_one(&pool)
    .await
    .unwrap();
    let j: Value = serde_json::from_str(&wishly_api::sealed::open_str(&snap).unwrap()).unwrap();
    let site_address: String = sqlx::query_scalar("SELECT site_address FROM wishlists WHERE id=$1")
        .bind(f.site.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(j["address"], json!(site_address));
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn paused_site_blocks_new_claims_and_pledges(pool: PgPool) {
    let f = fx(&pool).await;
    let q = relief_item(&pool, f.site.id, false).await;
    let c = relief_item(&pool, f.site.id, true).await;
    let u = user(&pool, "捐").await;
    grant(&pool, u.id, 100).await;
    let app = test_app(&pool);
    // 暫停前先有一筆認領，之後加量也要擋
    let (s, v) = claim(&pool, q, &u.token).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let cid = v["claim"]["id"].as_str().unwrap().to_string();
    for (st, code) in [("paused", "SITE_PAUSED"), ("closed", "SITE_CLOSED")] {
        sqlx::query("UPDATE wishlists SET site_status=$2 WHERE id=$1")
            .bind(f.site.id)
            .bind(st)
            .execute(&pool)
            .await
            .unwrap();
        let (s, v) = claim(&pool, q, &u.token).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some(code)));
        let (s, _, v) = pledge(&app, c, &u.token, 10).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some(code)));
        let (s, _, v) = call(
            &pool,
            "PATCH",
            &format!("/claims/{cid}"),
            &ck(&u.token),
            Some(json!({"qty":2})),
        )
        .await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some(code)));
    }
    assert_eq!(balance(&pool, u.id).await, 100);
}

/// 許可過期但每小時的 permit_expiry_tick 尚未把據點改成 paused：認捐、贊助、加量要比照暫停擋下，公開事件頁也不列出。
#[sqlx::test(migrations = "../../db/migrations")]
async fn expired_permit_blocks_site_before_tick(pool: PgPool) {
    let owner = user(&pool, "O").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e, slug) = event(&pool, o).await;
    let site = site(&pool, o, owner.id, e, p, "active").await;
    let q = relief_item(&pool, site.id, false).await;
    let c = relief_item(&pool, site.id, true).await;
    let u = user(&pool, "捐").await;
    grant(&pool, u.id, 100).await;
    let app = test_app(&pool);
    let (s, v) = claim(&pool, q, &u.token).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let cid = v["claim"]["id"].as_str().unwrap().to_string();
    let public_sites = || async {
        let (s, _, v) = call(
            &pool,
            "GET",
            &format!("/public/relief-events/{slug}"),
            &[],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["sites"].as_array().map_or(0, |a| a.len())
    };
    assert_eq!(public_sites().await, 1);

    // 台北日期的昨天到期；site_status 仍是 active（tick 還沒跑）
    sqlx::query("UPDATE org_permits SET valid_from = valid_from - interval '60 days', valid_to = (now() AT TIME ZONE 'Asia/Taipei')::date - 1 WHERE id=$1")
        .bind(p).execute(&pool).await.unwrap();
    let st: String = sqlx::query_scalar("SELECT site_status FROM wishlists WHERE id=$1")
        .bind(site.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "active");

    let (s, v) = claim(&pool, q, &u.token).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("SITE_PAUSED")),
        "{v}"
    );
    let (s, _, v) = pledge(&app, c, &u.token, 10).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("SITE_PAUSED")),
        "{v}"
    );
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/claims/{cid}"),
        &ck(&u.token),
        Some(json!({"qty":2})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("SITE_PAUSED")),
        "{v}"
    );
    assert_eq!(public_sites().await, 0);
    assert_eq!(balance(&pool, u.id).await, 100);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn paused_site_still_allows_cancel_decrease_and_withdraw(pool: PgPool) {
    let f = fx(&pool).await;
    let q = relief_item(&pool, f.site.id, false).await;
    let c = relief_item(&pool, f.site.id, true).await;
    let u = user(&pool, "捐").await;
    grant(&pool, u.id, 100).await;
    let app = test_app(&pool);
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/items/{q}/claims"),
        &[ck(&u.token), idem(Uuid::new_v4())].concat(),
        Some(json!({"qty":3})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let cid = v["claim"]["id"].as_str().unwrap().to_string();
    let (s, _, v) = pledge(&app, c, &u.token, 10).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let pid = v["contribution"]["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE wishlists SET site_status='paused' WHERE id=$1")
        .bind(f.site.id)
        .execute(&pool)
        .await
        .unwrap();
    let url = format!("/claims/{cid}");
    let hdr = ck(&u.token);
    assert_eq!(
        call(&pool, "PATCH", &url, &hdr, Some(json!({"qty":2})))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &url,
            &hdr,
            Some(json!({"status":"cancelled"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, _, v) = call(
        &pool,
        "DELETE",
        &format!("/contributions/{pid}"),
        &[ck(&u.token), idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
    assert_eq!(balance(&pool, u.id).await, 100);
}
