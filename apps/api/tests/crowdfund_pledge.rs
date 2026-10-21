//! P2-A 捐贈者端：認捐 / 撤回 / 轉投 / 錢包 / tick_funding。fixture 全以 SQL 建立，不依賴品項 API。
mod common;
use axum::{http::StatusCode, Router};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::{contributions, wallet, AppState};

fn test_app(pool: &PgPool) -> Router {
    Router::new()
        .nest("/api/v1", contributions::routes().merge(wallet::routes()))
        .with_state(AppState { pool: pool.clone() })
}

use common::{
    assert_all_reconciled, balance, crowdfund_item as cf_item, grant, pledge, Resp, TestUser,
};

/// 已啟用清單與加密收件地址；品項可直接用共用 SQL fixture。
async fn list_with_shipping(pool: &PgPool, owner: Uuid) -> Uuid {
    let w = common::wishlist(pool, owner, common::ListOpts::default())
        .await
        .id;
    sqlx::query("UPDATE wishlists SET title='寶寶清單' WHERE id=$1")
        .bind(w)
        .execute(pool)
        .await
        .unwrap();
    let sealed = |s: &str| wishly_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4)")
        .bind(w).bind(sealed("王小米")).bind(sealed("0912345678")).bind(sealed("台北市大安區某路 1 號")).execute(pool).await.unwrap();
    w
}

async fn item_row(pool: &PgPool, id: Uuid) -> (i64, String) {
    sqlx::query_as("SELECT pledged_points, funding_status::text FROM wishlist_items WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_happy_path(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser {
        id: donor,
        token: tok,
    } = common::user(&pool, "阿明").await;
    grant(&pool, donor, 1000).await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 500).await;

    let (st, h, b) = common::call_app(
        &app,
        "POST",
        &format!("/items/{item}/contributions"),
        &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "points": 200, "message": "恭喜", "is_anonymous": true })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert!(h["cache-control"].to_str().unwrap().contains("no-store"));
    assert_eq!(b["funded"], false);
    assert_eq!(b["wallet"]["balance"], 800);
    assert_eq!(b["item"]["pledged_points"], 200);
    assert_eq!(b["item"]["remaining_points"], 300);
    assert_eq!(b["item"]["progress_percent"], 40);
    assert_eq!(b["contribution"]["status"], "pledged");
    assert_eq!(b["contribution"]["is_anonymous"], true);
    assert_eq!(balance(&pool, donor).await, 800);
    let (delta, kind, after): (i64, String, i64) = sqlx::query_as(
        "SELECT delta, entry_type::text, balance_after FROM point_ledger WHERE ref_type = 'contribution' ORDER BY seq DESC LIMIT 1").fetch_one(&pool).await.unwrap();
    assert_eq!((delta, kind.as_str(), after), (-200, "pledge", 800));
    let name: String = sqlx::query_scalar("SELECT donor_name FROM contributions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "阿明");
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn exact_target_funds_and_captures(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let TestUser { id: b, token: tb } = common::user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    grant(&pool, b, 1000).await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 500).await;

    assert_eq!(pledge(&app, item, &ta, 300).await.0, StatusCode::CREATED);
    let (st, _, r) = pledge(&app, item, &tb, 200).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(r["funded"], true);
    assert_eq!(r["item"]["funding_status"], "funded");
    assert_eq!(r["item"]["progress_percent"], 100);
    assert_eq!(r["contribution"]["status"], "captured");
    assert_eq!(item_row(&pool, item).await, (500, "funded".into()));
    let captured: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM contributions WHERE status = 'captured' AND captured_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(captured, 2);
    let (amount, ost): (i64, String) =
        sqlx::query_as("SELECT amount, status::text FROM purchase_orders WHERE item_id = $1")
            .bind(item)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((amount, ost.as_str()), (500, "pending"));
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'crowdfund.funded'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 2);
    // 達標後再認捐 → ITEM_FUNDED
    let (st, _, e) = pledge(&app, item, &ta, 1).await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("ITEM_FUNDED"))
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn hard_cap_reports_remaining(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 500).await;
    assert_eq!(pledge(&app, item, &ta, 450).await.0, StatusCode::CREATED);
    let (st, _, e) = pledge(&app, item, &ta, 100).await;
    assert_eq!(
        (st, e["code"].as_str(), e["remaining_points"].as_i64()),
        (
            StatusCode::CONFLICT,
            Some("CROWDFUND_TARGET_EXCEEDED"),
            Some(50)
        )
    );
    assert_eq!(balance(&pool, a).await, 550);
    assert_eq!(item_row(&pool, item).await.0, 450);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_last_slot_only_one_wins(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 100).await;
    let mut users = vec![];
    for n in 0..6 {
        let TestUser { id, token: tok } = common::user(&pool, &format!("u{n}")).await;
        grant(&pool, id, 100).await;
        users.push((id, tok));
    }
    let tasks: Vec<_> = users
        .iter()
        .map(|(_, tok)| {
            let (app, tok) = (app.clone(), tok.clone());
            tokio::spawn(async move { pledge(&app, item, &tok, 60).await })
        })
        .collect();
    let mut ok = 0;
    for t in tasks {
        let (st, _, b) = t.await.unwrap();
        match st {
            StatusCode::CREATED => ok += 1,
            StatusCode::CONFLICT => assert!(
                ["CROWDFUND_TARGET_EXCEEDED", "ITEM_FUNDED"].contains(&b["code"].as_str().unwrap()),
                "{b}"
            ),
            other => panic!("unexpected {other} {b}"),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(item_row(&pool, item).await, (60, "open".into()));
    // 失敗者點數未被扣
    let mut spent = 0;
    for (id, _) in &users {
        let b = balance(&pool, *id).await;
        assert!(b == 100 || b == 40);
        if b == 40 {
            spent += 1;
        }
    }
    assert_eq!(spent, 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_exact_fill_never_oversells(pool: PgPool) {
    // 5 人各 30 點搶 100 點目標 → 最多 3 人成功（90），不會超額
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 100).await;
    let mut toks = vec![];
    for n in 0..5 {
        let TestUser { id, token: t } = common::user(&pool, &format!("u{n}")).await;
        grant(&pool, id, 100).await;
        toks.push(t);
    }
    let tasks: Vec<_> = toks
        .iter()
        .map(|t| {
            let (app, t) = (app.clone(), t.clone());
            tokio::spawn(async move { pledge(&app, item, &t, 30).await.0 })
        })
        .collect();
    let mut ok = 0;
    for t in tasks {
        if t.await.unwrap() == StatusCode::CREATED {
            ok += 1;
        }
    }
    assert_eq!(ok, 3);
    assert_eq!(item_row(&pool, item).await.0, 90);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_rejections(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 100).await;
    let wl = list_with_shipping(&pool, owner).await;
    let item = cf_item(&pool, wl, 500).await;
    let code = |r: &Resp| (r.0, r.2["code"].as_str().unwrap_or("").to_string());

    // 餘額不足：回 balance，且不留下認捐
    let r = pledge(&app, item, &ta, 101).await;
    assert_eq!(
        code(&r),
        (StatusCode::CONFLICT, "INSUFFICIENT_POINTS".into())
    );
    assert_eq!(r.2["balance"], 100);
    assert_eq!(item_row(&pool, item).await.0, 0);
    // 欄位驗證
    for p in [0, -5, 10_000_001] {
        assert_eq!(
            pledge(&app, item, &ta, p).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let r = common::call_app(
        &app,
        "POST",
        &format!("/items/{item}/contributions"),
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "points": 1, "message": "x".repeat(201) })),
    )
    .await;
    assert_eq!(r.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 未登入 / 缺 key
    let r = common::call_app(
        &app,
        "POST",
        &format!("/items/{item}/contributions"),
        &common::idem(Uuid::new_v4()),
        Some(json!({ "points": 1 })),
    )
    .await;
    assert_eq!(code(&r), (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".into()));
    let r = common::call_app(
        &app,
        "POST",
        &format!("/items/{item}/contributions"),
        &common::ck(&ta),
        Some(json!({ "points": 1 })),
    )
    .await;
    assert_eq!(
        code(&r),
        (StatusCode::BAD_REQUEST, "IDEMPOTENCY_KEY_REQUIRED".into())
    );
    // 不存在的品項
    assert_eq!(
        pledge(&app, Uuid::new_v4(), &ta, 1).await.0,
        StatusCode::NOT_FOUND
    );
    // 數量型品項
    let q: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(wl).fetch_one(&pool).await.unwrap();
    assert_eq!(
        code(&pledge(&app, q, &ta, 1).await),
        (StatusCode::CONFLICT, "FUNDING_MODE_MISMATCH".into())
    );
    // 已過截止但 tick 尚未跑
    sqlx::query(
        "UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        code(&pledge(&app, item, &ta, 1).await),
        (StatusCode::CONFLICT, "FUNDING_EXPIRED".into())
    );
    // 已 expired
    sqlx::query(
        "UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() WHERE id = $1",
    )
    .bind(item)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        code(&pledge(&app, item, &ta, 1).await),
        (StatusCode::CONFLICT, "FUNDING_EXPIRED".into())
    );
    // 凍結
    let open = cf_item(&pool, wl, 500).await;
    sqlx::query("UPDATE point_wallets SET status = 'frozen' WHERE user_id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        code(&pledge(&app, open, &ta, 1).await),
        (StatusCode::FORBIDDEN, "WALLET_FROZEN".into())
    );
    assert_eq!(balance(&pool, a).await, 100);
    // 清單已關閉
    sqlx::query("UPDATE point_wallets SET status = 'active'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE wishlists SET status = 'closed' WHERE id = $1")
        .bind(wl)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        code(&pledge(&app, open, &ta, 1).await),
        (StatusCode::CONFLICT, "WISHLIST_CLOSED".into())
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotent_replay_does_not_double_charge(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 500).await;
    let key = Uuid::new_v4();
    let uri = format!("/items/{item}/contributions");
    let (s1, h1, b1) = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(key)].concat(),
        Some(json!({ "points": 100 })),
    )
    .await;
    let (s2, h2, b2) = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(key)].concat(),
        Some(json!({ "points": 100 })),
    )
    .await;
    assert_eq!((s1, s2), (StatusCode::CREATED, StatusCode::CREATED));
    assert!(h1.get("idempotency-replayed").is_none());
    assert_eq!(h2["idempotency-replayed"], "true");
    assert_eq!(b1["contribution"]["id"], b2["contribution"]["id"]);
    assert_eq!(balance(&pool, a).await, 900);
    // 同 key 不同內容 → 衝突
    let (s3, _, b3) = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(key)].concat(),
        Some(json!({ "points": 50 })),
    )
    .await;
    assert_eq!(
        (s3, b3["code"].as_str()),
        (StatusCode::CONFLICT, Some("IDEMPOTENCY_CONFLICT"))
    );
    // 失敗不占 key：同 key 於餘額補足後可重試
    let k2 = Uuid::new_v4();
    let (s, ..) = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(k2)].concat(),
        Some(json!({ "points": 5000 })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE key = $1")
        .bind(k2)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_refunds_and_is_idempotent(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let TestUser { id: _, token: tb } = common::user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 500).await;
    let (_, _, r) = pledge(&app, item, &ta, 300).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();

    // 他人 403；不存在 404；未登入 401
    assert_eq!(
        common::call_app(
            &app,
            "DELETE",
            &format!("/contributions/{cid}"),
            &common::ck(&tb),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        common::call_app(
            &app,
            "DELETE",
            &format!("/contributions/{}", Uuid::new_v4()),
            &common::ck(&ta),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        common::call_app(&app, "DELETE", &format!("/contributions/{cid}"), &[], None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(balance(&pool, a).await, 700);

    let (st, _, b) = common::call_app(
        &app,
        "DELETE",
        &format!("/contributions/{cid}"),
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["contribution"]["status"], "released");
    assert_eq!(b["contribution"]["refunded_points"], 300);
    assert_eq!(b["wallet"]["balance"], 1000);
    assert_eq!(b["item"]["pledged_points"], 0);
    // 重複撤回：冪等，不重複退點
    let (st, _, b) = common::call_app(
        &app,
        "DELETE",
        &format!("/contributions/{cid}"),
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(
        (st, b["contribution"]["status"].as_str()),
        (StatusCode::OK, Some("released"))
    );
    assert_eq!(balance(&pool, a).await, 1000);
    let rel: i64 =
        sqlx::query_scalar("SELECT count(*) FROM point_ledger WHERE entry_type = 'release'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rel, 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_locked_when_captured_or_window_over(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let wl = list_with_shipping(&pool, owner).await;
    let item = cf_item(&pool, wl, 100).await;
    let (_, _, r) = pledge(&app, item, &ta, 100).await;
    assert_eq!(r["funded"], true);
    let cid = r["contribution"]["id"].as_str().unwrap();
    let (st, _, e) = common::call_app(
        &app,
        "DELETE",
        &format!("/contributions/{cid}"),
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("CONTRIBUTION_LOCKED"))
    );
    assert_eq!(balance(&pool, a).await, 900);

    // expired 且超過 7 天選擇期
    let item2 = cf_item(&pool, wl, 100).await;
    let (_, _, r) = pledge(&app, item2, &ta, 10).await;
    let cid2 = r["contribution"]["id"].as_str().unwrap();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '8 days' WHERE id = $1").bind(item2).execute(&pool).await.unwrap();
    let (st, _, e) = common::call_app(
        &app,
        "DELETE",
        &format!("/contributions/{cid2}"),
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("CONTRIBUTION_LOCKED"))
    );
    // expired 但仍在選擇期 → 可撤回
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '1 day' WHERE id = $1")
        .bind(item2)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        common::call_app(
            &app,
            "DELETE",
            &format!("/contributions/{cid2}"),
            &common::ck(&ta),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(balance(&pool, a).await, 900);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_within_choice_window(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let TestUser { id: _, token: tb } = common::user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    let wl = list_with_shipping(&pool, owner).await;
    let src = cf_item(&pool, wl, 500).await;
    let dst = cf_item(&pool, wl, 200).await;
    let other_wl = list_with_shipping(&pool, owner).await;
    let foreign = cf_item(&pool, other_wl, 200).await;
    let (_, _, r) = pledge(&app, src, &ta, 200).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();
    let uri = format!("/contributions/{cid}/reallocate");
    assert_eq!(
        common::call_app(
            &app,
            "PATCH",
            &format!("/contributions/{cid}"),
            &common::ck(&ta),
            Some(json!({ "visibility": "friends" }))
        )
        .await
        .0,
        StatusCode::OK
    );
    let go = |tok: &str, target: Uuid| {
        let (app, uri, tok) = (app.clone(), uri.clone(), tok.to_string());
        async move {
            common::call_app(
                &app,
                "POST",
                &uri,
                &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
                Some(json!({ "target_item_id": target })),
            )
            .await
        }
    };
    let not_allowed = |r: &Resp| {
        assert_eq!(
            (r.0, r.2["code"].as_str()),
            (StatusCode::CONFLICT, Some("REALLOCATION_NOT_ALLOWED")),
            "{}",
            r.2
        )
    };

    // 來源仍 open → 不可轉投
    not_allowed(&go(&ta, dst).await);
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day', funding_deadline = now() - interval '1 day' WHERE id = $1").bind(src).execute(&pool).await.unwrap();
    // 他人 403；跨清單 / 自己 / 不存在的目標 → 不可
    assert_eq!(go(&tb, dst).await.0, StatusCode::FORBIDDEN);
    not_allowed(&go(&ta, foreign).await);
    not_allowed(&go(&ta, src).await);
    not_allowed(&go(&ta, Uuid::new_v4()).await);
    // 無 Idempotency-Key
    assert_eq!(
        common::call_app(
            &app,
            "POST",
            &uri,
            &common::ck(&ta),
            Some(json!({ "target_item_id": dst }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    let before = balance(&pool, a).await;
    let ledger_before: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    let key = Uuid::new_v4();
    let body = json!({ "target_item_id": dst });
    let (st, _, b) = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(key)].concat(),
        Some(body.clone()),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert_eq!(b["funded"], true); // 200 點剛好填滿 dst
    assert_eq!(b["original"]["status"], "reallocated");
    assert_eq!(b["original"]["visibility"], "friends");
    assert_eq!(b["contribution"]["visibility"], "friends");
    assert_eq!(b["contribution"]["status"], "captured");
    assert_eq!(b["item"]["funding_status"], "funded");
    assert_eq!(item_row(&pool, src).await.0, 0);
    assert_eq!(item_row(&pool, dst).await, (200, "funded".into()));
    // 錢包不動、不寫 ledger
    assert_eq!(balance(&pool, a).await, before);
    let ledger_after: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ledger_before, ledger_after);
    let po: i64 = sqlx::query_scalar("SELECT count(*) FROM purchase_orders WHERE item_id = $1")
        .bind(dst)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(po, 1);
    // 重放同 key → 不會再轉一次
    let (st, h, _) = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(key)].concat(),
        Some(body),
    )
    .await;
    assert_eq!(
        (st, h["idempotency-replayed"].to_str().unwrap()),
        (StatusCode::CREATED, "true")
    );
    // 已轉投的不能再轉
    not_allowed(&go(&ta, dst).await);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_rejects_overflow_and_expired_window(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let wl = list_with_shipping(&pool, owner).await;
    let src = cf_item(&pool, wl, 500).await;
    let small = cf_item(&pool, wl, 100).await;
    let (_, _, r) = pledge(&app, src, &ta, 200).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day' WHERE id = $1").bind(src).execute(&pool).await.unwrap();
    let uri = format!("/contributions/{cid}/reallocate");
    let r = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "target_item_id": small })),
    )
    .await;
    assert_eq!(
        (r.0, r.2["code"].as_str()),
        (StatusCode::CONFLICT, Some("REALLOCATION_NOT_ALLOWED"))
    ); // 200 > 目標 100
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '8 days' WHERE id = $1")
        .bind(src)
        .execute(&pool)
        .await
        .unwrap();
    let big = cf_item(&pool, wl, 900).await;
    let r = common::call_app(
        &app,
        "POST",
        &uri,
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "target_item_id": big })),
    )
    .await;
    assert_eq!(
        (r.0, r.2["code"].as_str()),
        (StatusCode::CONFLICT, Some("REALLOCATION_NOT_ALLOWED"))
    ); // 選擇期已過
    assert_eq!(item_row(&pool, src).await.0, 200);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tick_expires_then_auto_refunds_once(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let TestUser { id: b, token: tb } = common::user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    grant(&pool, b, 1000).await;
    let wl = list_with_shipping(&pool, owner).await;
    let item = cf_item(&pool, wl, 500).await;
    let fresh = cf_item(&pool, wl, 500).await;
    pledge(&app, item, &ta, 100).await;
    pledge(&app, item, &tb, 150).await;
    pledge(&app, fresh, &ta, 50).await;

    // 未到期：不動
    assert_eq!(
        wishly_api::contributions::tick_funding(&pool)
            .await
            .unwrap(),
        (0, 0)
    );
    sqlx::query(
        "UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        wishly_api::contributions::tick_funding(&pool)
            .await
            .unwrap(),
        (1, 0)
    );
    let (pledged, fs) = item_row(&pool, item).await;
    assert_eq!((pledged, fs.as_str()), (250, "expired"));
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'funding.expired'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 2);
    // 重複執行：不再重複轉換 / 通知
    assert_eq!(
        wishly_api::contributions::tick_funding(&pool)
            .await
            .unwrap(),
        (0, 0)
    );
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'funding.expired'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 2);
    assert_eq!(balance(&pool, a).await, 850); // 選擇期內不退

    // 選擇期結束 → 自動退點
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '8 days' WHERE id = $1")
        .bind(item)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        wishly_api::contributions::tick_funding(&pool)
            .await
            .unwrap(),
        (0, 2)
    );
    assert_eq!(balance(&pool, a).await, 950); // 1000 - 50（fresh 仍 pledged）
    assert_eq!(balance(&pool, b).await, 1000);
    assert_eq!(item_row(&pool, item).await.0, 0);
    assert_eq!(item_row(&pool, fresh).await, (50, "open".into()));
    // 再跑不重複退點
    assert_eq!(
        wishly_api::contributions::tick_funding(&pool)
            .await
            .unwrap(),
        (0, 0)
    );
    assert_eq!(balance(&pool, a).await, 950);
    let rel: i64 =
        sqlx::query_scalar("SELECT count(*) FROM point_ledger WHERE entry_type = 'release'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rel, 2);
    // 並行執行安全
    sqlx::query(
        "UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(fresh)
    .execute(&pool)
    .await
    .unwrap();
    let (r1, r2) = tokio::join!(
        wishly_api::contributions::tick_funding(&pool),
        wishly_api::contributions::tick_funding(&pool)
    );
    assert_eq!(r1.unwrap().0 + r2.unwrap().0, 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_endpoint_held_points_and_pagination(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;

    // 沒有錢包
    let (st, h, b) = common::call_app(&app, "GET", "/wallet", &common::ck(&ta), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(
        (
            b["wallet"]["balance"].as_i64(),
            b["wallet"]["status"].as_str(),
            b["wallet"]["held_points"].as_i64()
        ),
        (Some(0), Some("active"), Some(0))
    );
    assert!(b["wallet"]["id"].is_null());
    assert_eq!(
        common::call_app(&app, "GET", "/wallet", &[], None).await.0,
        StatusCode::UNAUTHORIZED
    );

    grant(&pool, a, 1000).await;
    let wl = list_with_shipping(&pool, owner).await;
    let open = cf_item(&pool, wl, 500).await;
    let done = cf_item(&pool, wl, 100).await;
    pledge(&app, open, &ta, 120).await;
    pledge(&app, done, &ta, 100).await; // captured
    let (_, _, b) = common::call_app(&app, "GET", "/wallet", &common::ck(&ta), None).await;
    assert_eq!(b["wallet"]["balance"], 780);
    assert_eq!(b["wallet"]["held_points"], 220);
    assert_eq!(b["data"].as_array().unwrap().len(), 3);
    assert_eq!(b["data"][0]["entry_type"], "pledge"); // seq DESC
    assert_eq!(b["data"][2]["entry_type"], "grant");
    assert!(b["next_cursor"].is_null());
    // 部分退款後 held 只算實際花費
    sqlx::query("UPDATE contributions SET refunded_points = 10 WHERE item_id = $1")
        .bind(done)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE contribution_lot_allocations SET refunded_points = 10 WHERE contribution_id IN (SELECT id FROM contributions WHERE item_id = $1)")
        .bind(done)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, b) = common::call_app(&app, "GET", "/wallet", &common::ck(&ta), None).await;
    assert_eq!(b["wallet"]["held_points"], 210);

    // 分頁
    let (_, _, p1) = common::call_app(&app, "GET", "/wallet?limit=2", &common::ck(&ta), None).await;
    assert_eq!(p1["data"].as_array().unwrap().len(), 2);
    let cur = p1["next_cursor"].as_str().unwrap();
    let (_, _, p2) = common::call_app(
        &app,
        "GET",
        &format!("/wallet?limit=2&cursor={cur}"),
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert_eq!(p2["data"][0]["entry_type"], "grant");
    assert!(p2["next_cursor"].is_null());
    assert_eq!(
        common::call_app(&app, "GET", "/wallet?cursor=abc", &common::ck(&ta), None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let _ = a;
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_contributions_flags_and_filter(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let TestUser { id: _, token: tb } = common::user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    let wl = list_with_shipping(&pool, owner).await;
    let open = cf_item(&pool, wl, 500).await;
    let expired = cf_item(&pool, wl, 500).await;
    let late = cf_item(&pool, wl, 500).await;
    let done = cf_item(&pool, wl, 100).await;
    for it in [open, expired, late, done] {
        assert_eq!(
            pledge(&app, it, &ta, if it == done { 100 } else { 10 })
                .await
                .0,
            StatusCode::CREATED
        );
    }
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day' WHERE id = $1").bind(expired).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '9 days' WHERE id = $1").bind(late).execute(&pool).await.unwrap();

    let (st, h, b) =
        common::call_app(&app, "GET", "/wallet/contributions", &common::ck(&ta), None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(h["cache-control"], "private, no-store");
    let rows = b["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let by = |it: Uuid| {
        rows.iter()
            .find(|r| r["contribution"]["item_id"] == it.to_string())
            .unwrap()
    };
    let flags = |it: Uuid| {
        (
            by(it)["can_withdraw"].as_bool().unwrap(),
            by(it)["can_reallocate"].as_bool().unwrap(),
        )
    };
    assert_eq!(flags(open), (true, false));
    assert_eq!(flags(expired), (true, true));
    assert_eq!(flags(late), (false, false));
    assert_eq!(flags(done), (false, false));
    assert_eq!(by(open)["item"]["display_status"], "open");
    assert_eq!(by(done)["item"]["display_status"], "funded");
    assert_eq!(by(expired)["item"]["display_status"], "expired");
    assert!(by(expired)["item"]["reallocation_deadline"].is_string());
    assert!(by(open)["item"]["reallocation_deadline"].is_null());
    assert!(by(open)["wishlist"]["slug"].is_string());
    assert!(by(open)["item"].get("image_url").is_some());
    // 篩選 + 分頁
    let (_, _, f) = common::call_app(
        &app,
        "GET",
        "/wallet/contributions?status=captured",
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(f["data"].as_array().unwrap().len(), 1);
    assert_eq!(
        common::call_app(
            &app,
            "GET",
            "/wallet/contributions?status=bogus",
            &common::ck(&ta),
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, _, p1) = common::call_app(
        &app,
        "GET",
        "/wallet/contributions?limit=3",
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(p1["data"].as_array().unwrap().len(), 3);
    let cur = p1["next_cursor"].as_str().unwrap();
    let (_, _, p2) = common::call_app(
        &app,
        "GET",
        &format!("/wallet/contributions?limit=3&cursor={cur}"),
        &common::ck(&ta),
        None,
    )
    .await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert!(p2["next_cursor"].is_null());
    // 只看得到自己的
    let (_, _, other) =
        common::call_app(&app, "GET", "/wallet/contributions", &common::ck(&tb), None).await;
    assert_eq!(other["data"].as_array().unwrap().len(), 0);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_respects_list_visibility(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser {
        id: stranger,
        token: ts,
    } = common::user(&pool, "路人").await;
    let TestUser {
        id: friend,
        token: tf,
    } = common::user(&pool, "好友").await;
    for u in [stranger, friend] {
        grant(&pool, u, 1000).await;
    }
    common::befriend(&pool, owner, friend).await;
    let wl = list_with_shipping(&pool, owner).await;
    sqlx::query("UPDATE wishlists SET visibility = 'friends' WHERE id = $1")
        .bind(wl)
        .execute(&pool)
        .await
        .unwrap();
    let item = cf_item(&pool, wl, 500).await;
    let (st, _, b) = pledge(&app, item, &ts, 100).await;
    assert_eq!(
        (st, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("FRIENDS_ONLY"))
    );
    assert_eq!(balance(&pool, stranger).await, 1000);
    assert_eq!(pledge(&app, item, &tf, 100).await.0, StatusCode::CREATED);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn contribution_visibility_default_override_and_patch(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: owner_tok,
    } = common::user(&pool, "O").await;
    let TestUser {
        id: donor,
        token: tok,
    } = common::user(&pool, "D").await;
    grant(&pool, donor, 1000).await;
    let item = cf_item(&pool, list_with_shipping(&pool, owner).await, 500).await;
    sqlx::query("UPDATE users SET default_claim_visibility = 'friends' WHERE id = $1")
        .bind(donor)
        .execute(&pool)
        .await
        .unwrap();
    let r = pledge(&app, item, &tok, 100).await;
    assert_eq!(r.0, StatusCode::CREATED, "{}", r.2);
    assert_eq!(r.2["contribution"]["visibility"], "friends");
    let cid = r.2["contribution"]["id"].as_str().unwrap();
    let uri = format!("/contributions/{cid}");
    sqlx::query("UPDATE users SET default_claim_visibility = 'public' WHERE id = $1")
        .bind(donor)
        .execute(&pool)
        .await
        .unwrap();
    let rows = common::call_app(
        &app,
        "GET",
        "/wallet/contributions",
        &common::ck(&tok),
        None,
    )
    .await;
    assert_eq!(rows.2["data"][0]["contribution"]["visibility"], "friends");
    assert_eq!(
        pledge(&app, item, &tok, 100).await.2["contribution"]["visibility"],
        "public"
    );
    let r = common::call_app(
        &app,
        "POST",
        &format!("/items/{item}/contributions"),
        &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "points": 100, "visibility": "private" })),
    )
    .await;
    assert_eq!(r.2["contribution"]["visibility"], "private");
    for bad in [json!("bogus"), json!(42)] {
        assert_eq!(
            common::call_app(
                &app,
                "POST",
                &format!("/items/{item}/contributions"),
                &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
                Some(json!({ "points": 1, "visibility": bad }))
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            common::call_app(
                &app,
                "PATCH",
                &uri,
                &common::ck(&tok),
                Some(json!({ "visibility": bad }))
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for empty in [json!({}), json!({ "visibility": null })] {
        assert_eq!(
            common::call_app(&app, "PATCH", &uri, &common::ck(&tok), Some(empty))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        common::call_app(
            &app,
            "PATCH",
            &uri,
            &[],
            Some(json!({ "visibility": "public" }))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        common::call_app(
            &app,
            "PATCH",
            &uri,
            &common::ck(&owner_tok),
            Some(json!({ "visibility": "public" }))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        common::call_app(
            &app,
            "PATCH",
            &format!("/contributions/{}", Uuid::new_v4()),
            &common::ck(&tok),
            Some(json!({ "visibility": "public" }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let rows = common::call_app(
        &app,
        "GET",
        "/wallet/contributions",
        &common::ck(&tok),
        None,
    )
    .await;
    let unchanged = rows.2["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["contribution"]["id"] == cid)
        .unwrap();
    assert_eq!(unchanged["contribution"]["visibility"], "friends");
    let before = balance(&pool, donor).await;
    for vis in ["private", "friends", "public"] {
        let r = common::call_app(
            &app,
            "PATCH",
            &uri,
            &common::ck(&tok),
            Some(json!({ "visibility": vis })),
        )
        .await;
        assert_eq!(r.0, StatusCode::OK, "{}", r.2);
        assert!(r.1["cache-control"].to_str().unwrap().contains("no-store"));
        assert_eq!(r.2["contribution"]["visibility"], vis);
    }
    assert_eq!(balance(&pool, donor).await, before);
    // 已退回的歷史紀錄仍可調整隱私；不得異動點數。
    assert_eq!(
        common::call_app(&app, "DELETE", &uri, &common::ck(&tok), None)
            .await
            .0,
        StatusCode::OK
    );
    let r = common::call_app(
        &app,
        "PATCH",
        &uri,
        &common::ck(&tok),
        Some(json!({ "visibility": "private" })),
    )
    .await;
    assert_eq!(r.0, StatusCode::OK);
    assert_eq!(r.2["contribution"]["status"], "released");
    assert_eq!(balance(&pool, donor).await, before + 100);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn contribution_visibility_migration_preserves_existing_scope(pool: PgPool) {
    // Schema-only backfill fixture intentionally omits wallet debits/item totals;
    // assert_all_reconciled does not apply to this pre-0006 isolated database.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
    for name in [
        "0001_mvp.sql",
        "0002_password.sql",
        "0003_ratelimit.sql",
        "0004_crowdfund.sql",
        "0005_social.sql",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(
            std::fs::read_to_string(dir.join(name)).unwrap(),
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "O").await;
    // Pre-0006 schema has no closes_at fixture column.
    let wl = sqlx::query_scalar("INSERT INTO wishlists(owner_id,type,status,slug,title) VALUES($1,'registry','active',$2,'寶寶清單') RETURNING id")
        .bind(owner).bind(Uuid::new_v4().simple().to_string()[..10].to_string()).fetch_one(&pool).await.unwrap();
    let item = cf_item(&pool, wl, 1000).await;
    for vis in ["public", "friends", "private"] {
        let TestUser {
            id: donor,
            token: _,
        } = common::user(&pool, vis).await;
        sqlx::query(
            "UPDATE users SET default_claim_visibility = $2::text::share_level WHERE id = $1",
        )
        .bind(donor)
        .bind(vis)
        .execute(&pool)
        .await
        .unwrap();
        // pre-0022 schema 沒有 point_lots，不能用 grant helper
        sqlx::query("INSERT INTO point_wallets (user_id, balance) VALUES ($1, 100)")
            .bind(donor)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points)
            SELECT $1, $2, user_id, id, 'x', 10 FROM point_wallets WHERE user_id = $3")
            .bind(item).bind(wl).bind(donor).execute(&pool).await.unwrap();
    }
    let before: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(c) ORDER BY c.id) FROM contributions c")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        std::fs::read_to_string(dir.join("0006_contribution_visibility.sql")).unwrap(),
    ))
    .execute(&pool)
    .await
    .unwrap();
    let after: Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(c) - 'visibility' ORDER BY c.id) FROM contributions c",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after, before, "回填不得改動認捐原有欄位（含 updated_at）");
    let timestamp: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT max(updated_at) FROM contributions")
            .fetch_one(&pool)
            .await
            .unwrap();
    let timestamp_updates: bool = sqlx::query_scalar("UPDATE contributions SET message = '更新留言' WHERE id = (SELECT id FROM contributions LIMIT 1) RETURNING updated_at > $1")
        .bind(timestamp).fetch_one(&pool).await.unwrap();
    assert!(timestamp_updates, "回填後正常更新仍須觸發 updated_at");
    let scopes: Vec<(String, String)> = sqlx::query_as("SELECT u.default_claim_visibility::text, c.visibility::text FROM contributions c JOIN users u ON u.id = c.user_id ORDER BY u.default_claim_visibility")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        scopes,
        [
            ("public".into(), "public".into()),
            ("friends".into(), "friends".into()),
            ("private".into(), "private".into())
        ]
    );
    sqlx::query("UPDATE users SET default_claim_visibility = 'private'")
        .execute(&pool)
        .await
        .unwrap();
    let preserved: Vec<String> =
        sqlx::query_scalar("SELECT visibility::text FROM contributions ORDER BY visibility")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(preserved, ["friends", "private", "public"]);
    assert!(sqlx::query("UPDATE contributions SET visibility = NULL")
        .execute(&pool)
        .await
        .is_err());
    let fresh: String = sqlx::query_scalar("INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points)
        SELECT item_id, wishlist_id, user_id, wallet_id, donor_name, 1 FROM contributions LIMIT 1 RETURNING visibility::text")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(fresh, "private");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_rejected_when_wallet_frozen_or_list_not_pledgeable(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser {
        id: owner,
        token: _,
    } = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    common::befriend(&pool, owner, a).await;
    let wl = list_with_shipping(&pool, owner).await;
    sqlx::query("UPDATE wishlists SET visibility = 'friends' WHERE id = $1")
        .bind(wl)
        .execute(&pool)
        .await
        .unwrap();
    let src = cf_item(&pool, wl, 500).await;
    let dst = cf_item(&pool, wl, 200).await;
    let (_, _, r) = pledge(&app, src, &ta, 200).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();
    sqlx::query(
        "UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() WHERE id = $1",
    )
    .bind(src)
    .execute(&pool)
    .await
    .unwrap();
    // (情境, 破壞 SQL, 還原 SQL)；$1 = 甲（frozen / unfriended）或擁有者
    let cases: [(&str, &str, &str); 5] = [
        ("frozen", "UPDATE point_wallets SET status = 'frozen' WHERE user_id = $1", "UPDATE point_wallets SET status = 'active' WHERE user_id = $1"),
        ("hidden", "UPDATE wishlists SET moderation_status = 'hidden', moderation_reason = 'x' WHERE owner_id = $1", "UPDATE wishlists SET moderation_status = 'ok', moderation_reason = NULL WHERE owner_id = $1"),
        ("private", "UPDATE wishlists SET visibility = 'private' WHERE owner_id = $1", "UPDATE wishlists SET visibility = 'friends' WHERE owner_id = $1"),
        ("closed", "UPDATE wishlists SET closes_at = now() - interval '1 hour' WHERE owner_id = $1", "UPDATE wishlists SET closes_at = NULL WHERE owner_id = $1"),
        ("unfriended", "DELETE FROM friendships WHERE user_a = $1 OR user_b = $1", "SELECT 1 WHERE $1::uuid IS NOT NULL"),
    ];
    for (name, bad, undo) in cases {
        let who = if name == "frozen" || name == "unfriended" {
            a
        } else {
            owner
        };
        sqlx::query(bad).bind(who).execute(&pool).await.unwrap();
        let r = common::call_app(
            &app,
            "POST",
            &format!("/contributions/{cid}/reallocate"),
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "target_item_id": dst })),
        )
        .await;
        assert!(r.0.is_client_error(), "{name}: {} {}", r.0, r.2);
        if name == "frozen" {
            assert_eq!(r.2["code"], "WALLET_FROZEN");
        }
        sqlx::query(undo).bind(who).execute(&pool).await.unwrap();
    }
    let (captured, orders): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM contributions WHERE status = 'captured'), (SELECT count(*) FROM purchase_orders)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((captured, orders), (0, 0));
    let st: String =
        sqlx::query_scalar("SELECT status::text FROM contributions WHERE id = $1::uuid")
            .bind(&cid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(st, "pledged");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT pledged_points FROM wishlist_items WHERE id = $1")
            .bind(dst)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    common::assert_all_reconciled(&pool).await;
}
