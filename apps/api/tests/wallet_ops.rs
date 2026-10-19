//! 錢包後續（D55）：現金退款申請 / 營運處理、拒付處理函式、欠款結案、自動儲值設定。
mod common;
use axum::http::StatusCode;
use common::{
    assert_all_reconciled, call, ck, crowdfund_item, grant, idem, lots, pledge, seed_lot, test_app,
    topup_credit, TestUser,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::wallet_ops::{self, ChargedBack};

fn sources(l: &[(String, i64)]) -> Vec<(&str, i64)> {
    l.iter().map(|(s, n)| (s.as_str(), *n)).collect()
}

async fn refund(pool: &PgPool, token: &str, amount: i64, key: Uuid) -> common::Resp {
    call(
        pool,
        "POST",
        "/wallet/cash-refunds",
        &[ck(token), idem(key)].concat(),
        Some(json!({ "amount": amount })),
    )
    .await
}

async fn process(pool: &PgPool, staff: &str, id: &str, body: Value) -> common::Resp {
    call(
        pool,
        "PATCH",
        &format!("/admin/cash-refunds/{id}"),
        &ck(staff),
        Some(body),
    )
    .await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cash_refund_takes_only_refundable_lots_and_is_idempotent(pool: PgPool) {
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    grant(&pool, a, 50).await;
    seed_lot(&pool, a, "gift_card", 30, None).await;
    topup_credit(&pool, a, 100).await;

    // 缺 Idempotency-Key、金額不合法
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wallet/cash-refunds",
        &ck(&ta),
        Some(json!({ "amount": 10 })),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("IDEMPOTENCY_KEY_REQUIRED"))
    );
    let (s, ..) = refund(&pool, &ta, 0, Uuid::new_v4()).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    let key = Uuid::new_v4();
    let (s, _, b) = refund(&pool, &ta, 60, key).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(
        (
            b["request"]["status"].as_str(),
            b["wallet"]["balance"].as_i64()
        ),
        (Some("requested"), Some(120))
    );
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("adjustment", 50), ("gift_card", 30), ("topup", 40)],
        "現金退款只能扣 cash_refundable 批次"
    );
    let allocs: Value =
        sqlx::query_scalar("SELECT lot_allocations FROM cash_refund_requests LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(allocs.as_array().unwrap().len(), 1);
    assert_eq!(allocs[0]["points"], 60);
    // 重放：同回應、不再扣點
    let (s, h, b2) = refund(&pool, &ta, 60, key).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(h["idempotency-replayed"], "true");
    assert_eq!(b2["request"]["id"], b["request"]["id"]);
    assert_eq!(common::balance(&pool, a).await, 120);

    // balance 足夠（120）但可退現金只剩 40 → NOT_CASH_REFUNDABLE，整句不扣
    let (s, _, b) = refund(&pool, &ta, 41, Uuid::new_v4()).await;
    assert_eq!(
        (s, b["code"].as_str(), b["refundable_points"].as_i64()),
        (StatusCode::CONFLICT, Some("NOT_CASH_REFUNDABLE"), Some(40))
    );
    // 餘額不足
    let (s, _, b) = refund(&pool, &ta, 500, Uuid::new_v4()).await;
    assert_eq!(
        (s, b["code"].as_str(), b["balance"].as_i64()),
        (StatusCode::CONFLICT, Some("INSUFFICIENT_POINTS"), Some(120))
    );
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("adjustment", 50), ("gift_card", 30), ("topup", 40)]
    );
    // 凍結 → 403
    sqlx::query("UPDATE point_wallets SET status = 'frozen'")
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = refund(&pool, &ta, 10, Uuid::new_v4()).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("WALLET_FROZEN"))
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledged_points_cannot_be_cash_refunded(pool: PgPool) {
    let owner = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 100).await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    assert_eq!(
        pledge(&test_app(&pool), item, &ta, 80).await.0,
        StatusCode::CREATED
    );
    let (s, _, b) = refund(&pool, &ta, 50, Uuid::new_v4()).await;
    assert_eq!(
        (s, b["code"].as_str(), b["balance"].as_i64()),
        (StatusCode::CONFLICT, Some("INSUFFICIENT_POINTS"), Some(20))
    );
    let (s, ..) = refund(&pool, &ta, 20, Uuid::new_v4()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_processes_cash_refunds_and_rejection_restores_original_lots(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 100).await;
    topup_credit(&pool, a, 50).await;
    let (_, _, b) = refund(&pool, &ta, 120, Uuid::new_v4()).await; // 先進先出：100 + 20
    let id = b["request"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 0), ("topup", 30)]
    );

    // 權限
    let (s, _, b) = call(&pool, "GET", "/admin/cash-refunds", &ck(&ta), None).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("STAFF_ONLY"))
    );
    let (s, _, b) = call(
        &pool,
        "GET",
        "/admin/cash-refunds?status=requested",
        &ck(&staff.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["data"][0]["id"], id.as_str());
    assert_eq!(b["data"][0]["user"]["display_name"], "甲");
    assert_eq!(
        call(
            &pool,
            "GET",
            "/admin/cash-refunds?status=bogus",
            &ck(&staff.token),
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // 狀態機：requested 不能直接 completed；rejected 要 reason；不存在 404
    let (s, _, b) = process(&pool, &staff.token, &id, json!({ "status": "completed" })).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );
    let (s, ..) = process(&pool, &staff.token, &id, json!({ "status": "rejected" })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, ..) = process(
        &pool,
        &staff.token,
        &Uuid::new_v4().to_string(),
        json!({ "status": "processing" }),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _, b) = process(&pool, &staff.token, &id, json!({ "status": "processing" })).await;
    assert_eq!(
        (s, b["status"].as_str()),
        (StatusCode::OK, Some("processing"))
    );
    // 拒絕：點數退回「原批次」
    let (s, _, b) = process(
        &pool,
        &staff.token,
        &id,
        json!({ "status": "rejected", "reject_reason": "帳戶資料不符" }),
    )
    .await;
    assert_eq!(
        (s, b["status"].as_str()),
        (StatusCode::OK, Some("rejected"))
    );
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 100), ("topup", 50)],
        "rejected 必須依 lot_allocations 退回原批次"
    );
    assert_eq!(common::balance(&pool, a).await, 150);
    // 再拒絕一次 → 409，不會重複退
    let (s, ..) = process(
        &pool,
        &staff.token,
        &id,
        json!({ "status": "rejected", "reject_reason": "x" }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "cash refund state guard");
    assert_eq!(common::balance(&pool, a).await, 150);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'cash_refund.process' AND actor_type = 'staff'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 2);
    assert_all_reconciled(&pool).await;

    // 正常完成：requested → processing → completed，之後不能再拒絕
    let (_, _, b) = refund(&pool, &ta, 30, Uuid::new_v4()).await;
    let id2 = b["request"]["id"].as_str().unwrap().to_string();
    for st in ["processing", "completed"] {
        let (s, ..) = process(&pool, &staff.token, &id2, json!({ "status": st })).await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, ..) = process(
        &pool,
        &staff.token,
        &id2,
        json!({ "status": "rejected", "reject_reason": "x" }),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "completed 之後不可再拒絕（cash refund state guard）"
    );
    assert_eq!(common::balance(&pool, a).await, 120);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_rejections_restore_points_once(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 100).await;
    let (_, _, b) = refund(&pool, &ta, 100, Uuid::new_v4()).await;
    let id = b["request"]["id"].as_str().unwrap().to_string();
    let app = test_app(&pool);
    let go = || {
        let (app, token, id) = (app.clone(), staff.token.clone(), id.clone());
        async move {
            common::call_app(
                &app,
                "PATCH",
                &format!("/admin/cash-refunds/{id}"),
                &ck(&token),
                Some(json!({ "status": "rejected", "reject_reason": "重複" })),
            )
            .await
            .0
            .as_u16()
        }
    };
    let (x, y) = tokio::join!(go(), go());
    let mut codes = [x, y];
    codes.sort();
    assert_eq!(
        codes,
        [200, 409],
        "cash refund reject must restore exactly once"
    );
    assert_eq!(common::balance(&pool, a).await, 100);
    assert_all_reconciled(&pool).await;
}

// ---------- 拒付 ----------

#[sqlx::test(migrations = "../../db/migrations")]
async fn charge_back_recovers_remaining_lot_records_debt_and_is_idempotent(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let owner = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let topup = topup_credit(&pool, a, 100).await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    assert_eq!(
        pledge(&test_app(&pool), item, &ta, 30).await.0,
        StatusCode::CREATED
    ); // 批次剩 70，30 已用於認捐

    let mut tx = pool.begin().await.unwrap();
    let out = wallet_ops::charge_back(&mut tx, topup).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        out,
        ChargedBack::Applied {
            recovered: 70,
            debt: 30
        }
    );
    assert_eq!(common::balance(&pool, a).await, 0);
    assert_eq!(sources(&lots(&pool, a).await), [("topup", 0)]);
    let (wstatus, tstatus): (String, String) = sqlx::query_as("SELECT w.status::text, t.status::text FROM point_wallets w JOIN topups t ON t.wallet_id = w.id WHERE t.id = $1").bind(topup).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (wstatus.as_str(), tstatus.as_str()),
        ("frozen", "charged_back")
    );
    assert_all_reconciled(&pool).await;

    // 重送冪等：不再扣、不重複記欠款
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        wallet_ops::charge_back(&mut tx, topup).await.unwrap(),
        ChargedBack::Ignored,
        "charge_back must be idempotent"
    );
    tx.commit().await.unwrap();
    let debts: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_debts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(debts, 1);

    // 營運：列表 / 結案
    let (s, _, b) = call(&pool, "GET", "/admin/wallet-debts", &ck(&staff.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (
            b["data"].as_array().unwrap().len(),
            b["data"][0]["amount"].as_i64(),
            b["data"][0]["user"]["display_name"].as_str()
        ),
        (1, Some(30), Some("甲"))
    );
    let did = b["data"][0]["id"].as_str().unwrap().to_string();
    let patch = |body: Value, id: String| {
        let (pool, t) = (pool.clone(), staff.token.clone());
        async move {
            call(
                &pool,
                "PATCH",
                &format!("/admin/wallet-debts/{id}"),
                &ck(&t),
                Some(body),
            )
            .await
        }
    };
    assert_eq!(
        patch(json!({ "status": "settled" }), did.clone()).await.0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "note 必填"
    );
    assert_eq!(
        patch(json!({ "status": "open", "note": "x" }), did.clone())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, b) = patch(
        json!({ "status": "written_off", "note": "小額核銷" }),
        did.clone(),
    )
    .await;
    assert_eq!(
        (s, b["status"].as_str()),
        (StatusCode::OK, Some("written_off"))
    );
    let (s, _, b) = patch(json!({ "status": "settled", "note": "x" }), did).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")),
        "wallet debt state guard"
    );
    assert_eq!(
        patch(
            json!({ "status": "settled", "note": "x" }),
            Uuid::new_v4().to_string()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, _, b) = call(&pool, "GET", "/admin/wallet-debts", &ck(&staff.token), None).await;
    assert_eq!(b["data"].as_array().unwrap().len(), 0, "預設只列 open");
    let (_, _, b) = call(
        &pool,
        "GET",
        "/admin/wallet-debts?status=written_off",
        &ck(&staff.token),
        None,
    )
    .await;
    assert_eq!(b["data"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn dispute_freezes_wallet_win_restores_and_full_charge_back_has_no_debt(pool: PgPool) {
    let owner = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let topup = topup_credit(&pool, a, 100).await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    let app = test_app(&pool);

    let mut tx = pool.begin().await.unwrap();
    assert!(wallet_ops::dispute_open(&mut tx, topup).await.unwrap());
    assert!(!wallet_ops::dispute_open(&mut tx, topup).await.unwrap());
    tx.commit().await.unwrap();
    let (s, _, b) = pledge(&app, item, &ta, 10).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("WALLET_FROZEN"))
    );
    let (s, _, b) = refund(&pool, &ta, 10, Uuid::new_v4()).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("WALLET_FROZEN"))
    );

    // 爭議勝訴：恢復 paid 與 active
    let mut tx = pool.begin().await.unwrap();
    assert!(wallet_ops::dispute_won(&mut tx, topup).await.unwrap());
    assert!(!wallet_ops::dispute_won(&mut tx, topup).await.unwrap());
    tx.commit().await.unwrap();
    assert_eq!(pledge(&app, item, &ta, 10).await.0, StatusCode::CREATED);
    let st: String = sqlx::query_scalar("SELECT status::text FROM topups WHERE id = $1")
        .bind(topup)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "paid");

    // 再爭議 → 敗訴（已用 10，剩 90 可扣回）；用掉的 10 記欠款
    let mut tx = pool.begin().await.unwrap();
    wallet_ops::dispute_open(&mut tx, topup).await.unwrap();
    let out = wallet_ops::charge_back(&mut tx, topup).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        out,
        ChargedBack::Applied {
            recovered: 90,
            debt: 10
        }
    );
    assert_all_reconciled(&pool).await;

    // 全額未用的拒付：不記欠款
    let TestUser { id: b_id, .. } = common::user(&pool, "乙").await;
    let t2 = topup_credit(&pool, b_id, 50).await;
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        wallet_ops::charge_back(&mut tx, t2).await.unwrap(),
        ChargedBack::Applied {
            recovered: 50,
            debt: 0
        }
    );
    tx.commit().await.unwrap();
    let debts: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_debts WHERE topup_id = $1")
        .bind(t2)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(debts, 0);
    assert_all_reconciled(&pool).await;
}

// ---------- 自動儲值設定 ----------

#[sqlx::test(migrations = "../../db/migrations")]
async fn auto_topup_settings_crud_and_limits(pool: PgPool) {
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let put = |body: Value| {
        let (pool, t) = (pool.clone(), ta.clone());
        async move { call(&pool, "PUT", "/wallet/auto-topup", &ck(&t), Some(body)).await }
    };
    assert_eq!(
        call(&pool, "GET", "/wallet/auto-topup", &[], None).await.0,
        StatusCode::UNAUTHORIZED
    );
    // 預設值
    let (s, h, b) = call(&pool, "GET", "/wallet/auto-topup", &ck(&ta), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(
        (
            b["enabled"].as_bool(),
            b["threshold_points"].as_i64(),
            b["amount"].as_i64(),
            b["monthly_cap_amount"].as_i64(),
            b["used_this_month"].as_i64(),
            b["limit_reached"].as_bool()
        ),
        (
            Some(false),
            Some(500),
            Some(1000),
            Some(10_000),
            Some(0),
            Some(false)
        )
    );
    // 儲存設定（不啟用）
    let (s, _, b) =
        put(json!({ "threshold_points": 300, "amount": 2000, "monthly_cap_amount": 20000 })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        (
            b["threshold_points"].as_i64(),
            b["amount"].as_i64(),
            b["monthly_cap_amount"].as_i64()
        ),
        (Some(300), Some(2000), Some(20_000))
    );
    // 驗證
    for bad in [
        json!({ "amount": 50 }),
        json!({ "threshold_points": -1 }),
        json!({ "monthly_cap_amount": 0 }),
        json!({ "enabled": "yes" }),
    ] {
        assert_eq!(put(bad).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    // 單次金額大於月上限 → 409
    let (s, _, b) = put(json!({ "amount": 5000, "monthly_cap_amount": 3000 })).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("AUTO_TOPUP_LIMIT_REACHED"))
    );
    // 啟用需已綁卡；金流商未接入，綁卡（prime）→ 503
    let (s, ..) = put(json!({ "enabled": true })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, b) = put(json!({ "enabled": true, "prime": "test_x" })).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("TOPUP_DISABLED"))
    );
    sqlx::query("UPDATE wallet_auto_topups SET card_token_ref = 'tok', card_last4 = '4242' WHERE user_id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = put(json!({ "enabled": true })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        (b["enabled"].as_bool(), b["card_last4"].as_str()),
        (Some(true), Some("4242"))
    );
    assert!(b.get("card_token_ref").is_none(), "不回傳 card token");

    // 當月自動儲值累計 / 月上限
    let w: Uuid = sqlx::query_scalar("INSERT INTO point_wallets (user_id) VALUES ($1) ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id RETURNING id").bind(a).fetch_one(&pool).await.unwrap();
    for (n, status) in [(15_000i64, "paid"), (1_000, "pending"), (9_999, "failed")] {
        sqlx::query("INSERT INTO topups (wallet_id, merchant_trade_no, provider, method, amount, status, provider_txn_id, paid_at, expires_at, is_auto)
                     VALUES ($1, $2, 'tappay', 'card', $3, $4::topup_status, CASE WHEN $4 = 'paid' THEN $2 END, CASE WHEN $4 = 'paid' THEN now() END, now(), true)")
            .bind(w).bind(Uuid::new_v4().to_string()).bind(n).bind(status).execute(&pool).await.unwrap();
    }
    let (_, _, b) = call(&pool, "GET", "/wallet/auto-topup", &ck(&ta), None).await;
    assert_eq!(b["used_this_month"], 16_000);
    assert_eq!(b["limit_reached"], false); // 16000 + 2000 <= 20000
    let (_, _, b) = put(json!({ "amount": 5000 })).await;
    assert_eq!(b["limit_reached"], true); // 16000 + 5000 > 20000

    // DELETE → 204，回到預設
    let (s, ..) = call(&pool, "DELETE", "/wallet/auto-topup", &ck(&ta), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, b) = call(&pool, "GET", "/wallet/auto-topup", &ck(&ta), None).await;
    assert_eq!(
        (b["enabled"].as_bool(), b["card_last4"].is_null()),
        (Some(false), true)
    );
}
