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
    // 欠款未結清不得解凍；結案後才可
    let wid: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(a)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (wpath, wck) = (format!("/admin/wallets/{wid}"), ck(&staff.token));
    let unfreeze = || {
        call(
            &pool,
            "PATCH",
            &wpath,
            &wck,
            Some(json!({ "status": "active", "reason": "測試" })),
        )
    };
    let (s, _, b) = unfreeze().await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("WALLET_DEBT_OPEN"))
    );
    assert_eq!(wallet_status(&pool, a).await, "frozen");
    let (s, _, b) = patch(
        json!({ "status": "written_off", "note": "小額核銷" }),
        did.clone(),
    )
    .await;
    assert_eq!(
        (s, b["status"].as_str()),
        (StatusCode::OK, Some("written_off"))
    );
    assert_eq!(unfreeze().await.0, StatusCode::OK);
    assert_eq!(wallet_status(&pool, a).await, "active");
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

/// 退回點數抵銷拒付欠款：topup 100，認捐 pledges 各筆後拒付；回傳 (甲, token, 品項, 認捐 id 們, topup, 錢包)。
async fn charged_back_with_pledges(
    pool: &PgPool,
    pledges: &[i64],
) -> (Uuid, String, Vec<Uuid>, Uuid, Uuid) {
    let owner = common::user(pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(pool, "甲").await;
    let topup = topup_credit(pool, a, 100).await;
    let wl = common::wishlist(pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(pool, wl, 1000).await;
    let app = test_app(pool);
    let mut cids = vec![];
    for n in pledges {
        let (s, _, r) = pledge(&app, item, &ta, *n).await;
        assert_eq!(s, StatusCode::CREATED);
        cids.push(r["contribution"]["id"].as_str().unwrap().parse().unwrap());
    }
    let mut tx = pool.begin().await.unwrap();
    wallet_ops::charge_back(&mut tx, topup).await.unwrap();
    tx.commit().await.unwrap();
    (a, ta, cids, topup, item)
}

async fn withdraw(pool: &PgPool, token: &str, cid: Uuid) {
    let (s, ..) = call(
        pool,
        "DELETE",
        &format!("/contributions/{cid}"),
        &ck(token),
        None,
    )
    .await;
    assert!(s.is_success(), "withdraw failed: {s}");
}

/// (balance, topup 批次剩餘, recovered_points, 欠款狀態)
async fn debt_state(pool: &PgPool, user: Uuid, topup: Uuid) -> (i64, i64, i64, String) {
    let (rec, st): (i64, String) = sqlx::query_as(
        "SELECT recovered_points, status::text FROM wallet_debts WHERE topup_id = $1",
    )
    .bind(topup)
    .fetch_one(pool)
    .await
    .unwrap();
    let rem = lots(pool, user).await[0].1;
    (common::balance(pool, user).await, rem, rec, st)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdrawn_pledge_after_charge_back_offsets_debt_instead_of_reviving_points(pool: PgPool) {
    let (a, ta, cids, topup, _) = charged_back_with_pledges(&pool, &[70]).await;
    assert_eq!(debt_state(&pool, a, topup).await, (0, 0, 0, "open".into()));
    withdraw(&pool, &ta, cids[0]).await;
    assert_eq!(
        debt_state(&pool, a, topup).await,
        (0, 0, 70, "settled".into()),
        "returned points must offset the charge-back debt"
    );
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entry_type::text, delta FROM point_ledger WHERE ref_type IN ('contribution', 'wallet_debt') AND entry_type::text IN ('release', 'adjustment') ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert!(rows.contains(&("release".into(), 70)) && rows.contains(&("adjustment".into(), -70)));
    assert_all_reconciled(&pool).await;
    // 欠款自動結清 → 可解凍
    let staff = common::staff(&pool, "營運").await;
    let wid: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(a)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/wallets/{wid}"),
        &ck(&staff.token),
        Some(json!({ "status": "active", "reason": "測試" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn partial_offset_keeps_debt_open_and_wallet_frozen(pool: PgPool) {
    let (a, ta, cids, topup, _) = charged_back_with_pledges(&pool, &[40, 30]).await;
    withdraw(&pool, &ta, cids[0]).await;
    assert_eq!(debt_state(&pool, a, topup).await, (0, 0, 40, "open".into()));
    let staff = common::staff(&pool, "營運").await;
    let wid: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(a)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/wallets/{wid}"),
        &ck(&staff.token),
        Some(json!({ "status": "active", "reason": "測試" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn settled_debt_lets_returned_points_credit_normally(pool: PgPool) {
    let (a, ta, cids, topup, _) = charged_back_with_pledges(&pool, &[70]).await;
    sqlx::query("UPDATE wallet_debts SET status = 'settled', resolved_at = now(), note = '已繳清'")
        .execute(&pool)
        .await
        .unwrap();
    withdraw(&pool, &ta, cids[0]).await;
    assert_eq!(
        debt_state(&pool, a, topup).await,
        (70, 70, 0, "settled".into()),
        "settled debt must not offset returned points"
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn written_off_debt_is_still_offset_by_returned_points(pool: PgPool) {
    let (a, ta, cids, topup, _) = charged_back_with_pledges(&pool, &[70]).await;
    sqlx::query(
        "UPDATE wallet_debts SET status = 'written_off', resolved_at = now(), note = '核銷'",
    )
    .execute(&pool)
    .await
    .unwrap();
    withdraw(&pool, &ta, cids[0]).await;
    assert_eq!(
        debt_state(&pool, a, topup).await,
        (0, 0, 70, "written_off".into())
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn rejected_cash_refund_after_charge_back_offsets_debt(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let topup = topup_credit(&pool, a, 100).await;
    let (s, _, b) = refund(&pool, &ta, 60, Uuid::new_v4()).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = b["request"]["id"].as_str().unwrap().to_string();
    let mut tx = pool.begin().await.unwrap();
    wallet_ops::charge_back(&mut tx, topup).await.unwrap(); // 扣回 40，欠 60
    tx.commit().await.unwrap();
    let (s, ..) = process(
        &pool,
        &staff.token,
        &id,
        json!({ "status": "rejected", "reject_reason": "測試" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        debt_state(&pool, a, topup).await,
        (0, 0, 60, "settled".into())
    );
    assert_all_reconciled(&pool).await;
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

// ---------- 營運拒付入口（D57）----------

async fn dispute(pool: &PgPool, token: &str, topup: Uuid, key: Uuid, body: Value) -> common::Resp {
    call(
        pool,
        "POST",
        &format!("/admin/topups/{topup}/dispute"),
        &[ck(token), idem(key)].concat(),
        Some(body),
    )
    .await
}

async fn topup_status(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM topups WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn wallet_status(pool: &PgPool, user: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM point_wallets WHERE user_id = $1")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type = 'staff' AND action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_dispute_open_won_lost_flow_is_idempotent_and_audited(pool: PgPool) {
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
    );

    // open：凍結；同 key 重放帶 Idempotency-Replayed；不同 key 重送 applied=false 且不重複 audit
    let k = Uuid::new_v4();
    let open = json!({ "action": "open", "note": "銀行通知" });
    let (s, _, b) = dispute(&pool, &staff.token, topup, k, open.clone()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        (b["status"].as_str(), b["applied"].as_bool()),
        (Some("disputed"), Some(true))
    );
    assert_eq!(wallet_status(&pool, a).await, "frozen");
    let (s, h, b2) = dispute(&pool, &staff.token, topup, k, open).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b2, b);
    assert_eq!(
        h.get("idempotency-replayed").map(|v| v.as_bytes()),
        Some(&b"true"[..])
    );
    let again = json!({ "action": "open" });
    let (s, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), again).await;
    assert_eq!(
        (s, b["status"].as_str(), b["applied"].as_bool()),
        (StatusCode::OK, Some("disputed"), Some(false))
    );
    assert_eq!(audit_count(&pool, "topup.dispute_open").await, 1);
    // GET /wallet 讓前端顯示凍結提示
    let (_, _, w) = call(&pool, "GET", "/wallet", &ck(&ta), None).await;
    assert_eq!(w["wallet"]["status"], "frozen");

    // won：恢復
    let won = json!({ "action": "won" });
    let (s, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), won.clone()).await;
    assert_eq!(
        (s, b["status"].as_str(), b["applied"].as_bool()),
        (StatusCode::OK, Some("paid"), Some(true))
    );
    assert_eq!(wallet_status(&pool, a).await, "active");
    let (_, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), won).await;
    assert_eq!(b["applied"].as_bool(), Some(false));
    assert_eq!(audit_count(&pool, "topup.dispute_won").await, 1);

    // 再 open → lost：扣回批次剩餘 70，已認捐的 30 記欠款
    dispute(
        &pool,
        &staff.token,
        topup,
        Uuid::new_v4(),
        json!({ "action": "open" }),
    )
    .await;
    let lost = json!({ "action": "lost", "note": "敗訴" });
    let (s, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), lost).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        (
            b["status"].as_str(),
            b["applied"].as_bool(),
            b["recovered"].as_i64(),
            b["debt"].as_i64()
        ),
        (Some("charged_back"), Some(true), Some(70), Some(30))
    );
    assert_eq!(wallet_status(&pool, a).await, "frozen");
    assert_eq!(common::balance(&pool, a).await, 0);
    let lost2 = json!({ "action": "lost" });
    let (s, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), lost2).await;
    assert_eq!(
        (s, b["applied"].as_bool(), b.get("debt").is_none()),
        (StatusCode::OK, Some(false), true)
    );
    let debts: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_debts WHERE amount = 30")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(debts, 1);
    // 終態不可再爭議 / 勝訴
    for action in ["open", "won"] {
        let body = json!({ "action": action });
        let (s, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), body).await;
        assert_eq!(
            (s, b["code"].as_str()),
            (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")),
            "charged_back 後 {action}"
        );
    }

    // audit_logs：staff 操作各一筆，diff 含 note 與結果
    assert_eq!(audit_count(&pool, "topup.dispute_lost").await, 1);
    let diff: Value =
        sqlx::query_scalar("SELECT diff FROM audit_logs WHERE action = 'topup.dispute_lost'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (
            diff["note"].as_str(),
            diff["recovered"].as_i64(),
            diff["debt"].as_i64()
        ),
        (Some("敗訴"), Some(70), Some(30))
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_dispute_rejects_bad_state_input_and_non_staff(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let paid = topup_credit(&pool, a, 100).await;
    let wallet: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(a)
        .fetch_one(&pool)
        .await
        .unwrap();
    let pending = Uuid::new_v4();
    sqlx::query("INSERT INTO topups (id, wallet_id, merchant_trade_no, provider, method, amount, expires_at) VALUES ($1, $2, $3, 'ecpay', 'card', 100, now())")
        .bind(pending).bind(wallet).bind(pending.to_string()).execute(&pool).await.unwrap();

    for action in ["open", "won", "lost"] {
        let body = json!({ "action": action });
        let (s, _, b) = dispute(&pool, &staff.token, pending, Uuid::new_v4(), body).await;
        assert_eq!(
            (s, b["code"].as_str()),
            (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")),
            "pending 不可 {action}"
        );
    }
    assert_eq!(topup_status(&pool, pending).await, "pending");
    assert_eq!(wallet_status(&pool, a).await, "active");

    let nf = Uuid::new_v4();
    let (s, ..) = dispute(
        &pool,
        &staff.token,
        nf,
        Uuid::new_v4(),
        json!({ "action": "open" }),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // 輸入驗證
    for body in [
        json!({ "action": "x" }),
        json!({}),
        json!({ "action": "open", "note": "字".repeat(501) }),
    ] {
        let (s, ..) = dispute(&pool, &staff.token, paid, Uuid::new_v4(), body).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let path = format!("/admin/topups/{paid}/dispute");
    let body = json!({ "action": "open" });
    let (s, _, b) = call(&pool, "POST", &path, &ck(&staff.token), Some(body.clone())).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("IDEMPOTENCY_KEY_REQUIRED"))
    );
    // 同 key 不同內容
    let k = Uuid::new_v4();
    assert_eq!(
        dispute(&pool, &staff.token, paid, k, body.clone()).await.0,
        StatusCode::OK
    );
    let other = json!({ "action": "lost" });
    let (s, _, b) = dispute(&pool, &staff.token, paid, k, other).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("IDEMPOTENCY_CONFLICT"))
    );
    // 權限
    let (s, _, b) = dispute(&pool, &ta, paid, Uuid::new_v4(), body.clone()).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("STAFF_ONLY"))
    );
    let (s, ..) = call(&pool, "POST", &path, &idem(Uuid::new_v4()), Some(body)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        call(&pool, "GET", "/admin/topups", &ck(&ta), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&pool, "GET", "/admin/topups", &[], None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn paid_topup_can_be_charged_back_directly(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, .. } = common::user(&pool, "甲").await;
    let topup = topup_credit(&pool, a, 100).await;
    let body = json!({ "action": "lost" });
    let (s, _, b) = dispute(&pool, &staff.token, topup, Uuid::new_v4(), body).await;
    assert_eq!(
        (s, b["recovered"].as_i64(), b["debt"].as_i64()),
        (StatusCode::OK, Some(100), Some(0))
    );
    assert_eq!(wallet_status(&pool, a).await, "frozen");
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_topup_list_filters_and_paginates(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let mut ids = vec![];
    for name in ["甲", "乙", "丙"] {
        let u = common::user(&pool, name).await;
        ids.push(topup_credit(&pool, u.id, 100).await);
    }
    let mut tx = pool.begin().await.unwrap();
    wallet_ops::dispute_open(&mut tx, ids[1]).await.unwrap();
    tx.commit().await.unwrap();

    let get = |q: String| {
        let (pool, t) = (pool.clone(), staff.token.clone());
        async move { call(&pool, "GET", &format!("/admin/topups{q}"), &ck(&t), None).await }
    };
    let (s, h, b) = get("?status=disputed".into()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get("cache-control").unwrap(), "private, no-store");
    let d = &b["data"];
    assert_eq!(d.as_array().unwrap().len(), 1);
    assert_eq!(
        (
            d[0]["id"].as_str(),
            d[0]["status"].as_str(),
            d[0]["amount"].as_i64(),
            d[0]["method"].as_str(),
            d[0]["provider"].as_str(),
            d[0]["user"]["display_name"].as_str(),
            d[0]["wallet"]["status"].as_str()
        ),
        (
            Some(ids[1].to_string().as_str()),
            Some("disputed"),
            Some(100),
            Some("card"),
            Some("ecpay"),
            Some("乙"),
            Some("frozen")
        )
    );
    assert!(d[0]["user"]["email"].is_string() && d[0].get("paid_at").is_some());
    let mtn: String = sqlx::query_scalar("SELECT merchant_trade_no FROM topups WHERE id=$1")
        .bind(ids[1])
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(d[0]["merchant_trade_no"].as_str(), Some(mtn.as_str()));
    assert!(b["next_cursor"].is_null());

    // 分頁：limit=2 → 兩頁共 3 筆、不重複；limit 越界被夾住
    let (_, _, p1) = get("?limit=2".into()).await;
    assert_eq!(p1["data"].as_array().unwrap().len(), 2);
    let cur = p1["next_cursor"].as_str().unwrap().to_string();
    let (_, _, p2) = get(format!("?limit=2&cursor={cur}")).await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert!(p2["next_cursor"].is_null());
    let mut seen: Vec<&str> = p1["data"]
        .as_array()
        .unwrap()
        .iter()
        .chain(p2["data"].as_array().unwrap())
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 3);
    let len = |v: Value| v["data"].as_array().unwrap().len();
    assert_eq!(len(get("?limit=0".into()).await.2), 1);
    assert_eq!(len(get("?limit=9999".into()).await.2), 3);
    assert_eq!(len(get("?status=paid".into()).await.2), 2);
    assert_eq!(
        get("?status=bogus".into()).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

async fn topup_get(pool: &PgPool, token: &str, q: &str) -> (StatusCode, Value) {
    let (s, _, b) = call(pool, "GET", &format!("/admin/topups{q}"), &ck(token), None).await;
    (s, b)
}

fn topup_ids(b: &Value) -> Vec<String> {
    b["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topups_filter_by_method_and_user(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let a = common::user(&pool, "甲").await;
    let b = common::user(&pool, "乙").await;
    let ta = topup_credit(&pool, a.id, 100).await;
    let tb = topup_credit(&pool, b.id, 100).await;
    sqlx::query("UPDATE topups SET method='line_pay' WHERE id=$1")
        .bind(tb)
        .execute(&pool)
        .await
        .unwrap();
    let (_, r) = topup_get(&pool, &staff.token, "?method=line_pay").await;
    assert_eq!(topup_ids(&r), [tb.to_string()]);
    let (_, r) = topup_get(&pool, &staff.token, "?method=card").await;
    assert_eq!(topup_ids(&r), [ta.to_string()]);
    let (_, r) = topup_get(&pool, &staff.token, &format!("?user_id={}", b.id)).await;
    assert_eq!(topup_ids(&r), [tb.to_string()]);
    let (_, r) = topup_get(&pool, &staff.token, "?q=%20").await;
    assert_eq!(topup_ids(&r).len(), 2, "空白 q 視為不篩選");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topups_q_is_literal(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let a = common::user(&pool, "甲").await;
    topup_credit(&pool, a.id, 100).await;
    let (_, r) = topup_get(&pool, &staff.token, "?q=%25").await;
    assert!(topup_ids(&r).is_empty(), "q=% 不可當萬用字元");
    let (_, r) = topup_get(&pool, &staff.token, "?q=%E7%94%B2").await;
    assert_eq!(topup_ids(&r).len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topups_q_cursor_paginates(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let mut want = vec![];
    for n in ["命中一", "命中二", "命中三"] {
        let u = common::user(&pool, n).await;
        want.push(topup_credit(&pool, u.id, 100).await.to_string());
    }
    let other = common::user(&pool, "路人").await;
    topup_credit(&pool, other.id, 100).await;
    let q = "%E5%91%BD%E4%B8%AD";
    let mut got = vec![];
    let mut cur = String::new();
    loop {
        let (_, r) = topup_get(&pool, &staff.token, &format!("?q={q}&limit=1{cur}")).await;
        got.extend(topup_ids(&r));
        match r["next_cursor"].as_str() {
            Some(c) => cur = format!("&cursor={c}"),
            None => break,
        }
    }
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topups_deleted_user_still_listed(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let a = common::user(&pool, "甲").await;
    let t = topup_credit(&pool, a.id, 100).await;
    sqlx::query("UPDATE users SET display_name='已刪除的使用者', email=NULL WHERE id=$1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, r) = topup_get(&pool, &staff.token, &format!("?user_id={}", a.id)).await;
    assert_eq!(topup_ids(&r), [t.to_string()]);
    // email 為 NULL 時姓名仍要能命中
    let (_, r) = topup_get(&pool, &staff.token, "?q=%E5%B7%B2%E5%88%AA%E9%99%A4").await;
    assert_eq!(topup_ids(&r), [t.to_string()]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topups_bad_method_422(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let (s, _) = topup_get(&pool, &staff.token, "?method=bogus").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn my_cash_refund_list_is_own_newest_first_and_hides_internals(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    let TestUser {
        id: b_id,
        token: tb,
    } = common::user(&pool, "乙").await;
    topup_credit(&pool, a, 100).await;
    topup_credit(&pool, b_id, 100).await;
    let (_, _, r1) = refund(&pool, &ta, 10, Uuid::new_v4()).await;
    let (_, _, r2) = refund(&pool, &ta, 20, Uuid::new_v4()).await;
    refund(&pool, &tb, 30, Uuid::new_v4()).await;
    let id1 = r1["request"]["id"].as_str().unwrap().to_string();
    let reject = json!({ "status": "rejected", "reject_reason": "帳戶有誤" });
    assert_eq!(
        process(&pool, &staff.token, &id1, reject).await.0,
        StatusCode::OK
    );

    let (s, h, b) = call(&pool, "GET", "/wallet/cash-refunds", &ck(&ta), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get("cache-control").unwrap(), "private, no-store");
    let rs = b["requests"].as_array().unwrap();
    assert_eq!(rs.len(), 2, "看不到他人的申請");
    assert_eq!(rs[0]["id"], r2["request"]["id"], "最新在前");
    assert_eq!(
        (
            rs[0]["amount"].as_i64(),
            rs[0]["status"].as_str(),
            rs[0]["completed_at"].is_null(),
            rs[0]["reject_reason"].is_null()
        ),
        (Some(20), Some("requested"), true, true)
    );
    assert_eq!(
        (
            rs[1]["status"].as_str(),
            rs[1]["reject_reason"].as_str(),
            rs[1]["completed_at"].is_string()
        ),
        (Some("rejected"), Some("帳戶有誤"), true)
    );
    let mut keys: Vec<&String> = rs[0].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "amount",
            "completed_at",
            "created_at",
            "id",
            "reject_reason",
            "status"
        ]
    );
    // limit 邊界
    let n = |q: &'static str| {
        let (pool, t) = (pool.clone(), ta.clone());
        async move {
            let uri = format!("/wallet/cash-refunds{q}");
            call(&pool, "GET", &uri, &ck(&t), None).await.2["requests"]
                .as_array()
                .unwrap()
                .len()
        }
    };
    assert_eq!(
        (
            n("?limit=1").await,
            n("?limit=0").await,
            n("?limit=999").await
        ),
        (1, 1, 2)
    );
    assert_eq!(
        call(&pool, "GET", "/wallet/cash-refunds", &[], None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // 沒有申請的使用者拿到空陣列
    let c = common::user(&pool, "丙").await;
    let (s, _, b) = call(&pool, "GET", "/wallet/cash-refunds", &ck(&c.token), None).await;
    assert_eq!(
        (s, b["requests"].as_array().map(Vec::len)),
        (StatusCode::OK, Some(0))
    );
}

/// 決定性併發：持鎖交易模擬「另一個正在處理 lost 的營運」；請求必須卡在 topups 列鎖上，
/// 等它提交後讀到 charged_back → applied=false，只扣回一次、欠款一筆。
#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_lost_waits_on_topup_row_lock_and_charges_back_once(pool: PgPool) {
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
    );

    let mut holder = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    // 持鎖者先處理 lost（FOR UPDATE 在 charge_back 內取得並持有到 commit）
    let out = wallet_ops::charge_back(&mut holder, topup).await.unwrap();
    assert_eq!(
        out,
        ChargedBack::Applied {
            recovered: 70,
            debt: 30
        }
    );

    let (p, t) = (pool.clone(), staff.token.clone());
    let req = tokio::spawn(async move {
        dispute(&p, &t, topup, Uuid::new_v4(), json!({ "action": "lost" })).await
    });
    let blocked = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        common::wait_for_blocked(&pool, pid, 1),
    )
    .await;
    assert!(
        blocked.is_ok(),
        "lost request did not block on the topups row lock"
    );
    holder.commit().await.unwrap();

    let (status, _, b) = req.await.unwrap();
    // 只比狀態碼與欄位：失敗時 body 可能帶 INTERNAL_ERROR，mutate.py 會誤判為基礎設施錯誤
    assert_eq!(
        status,
        StatusCode::OK,
        "blocked lost request failed after concurrent charge_back"
    );
    assert_eq!(
        b["applied"].as_bool(),
        Some(false),
        "charge_back applied twice"
    );
    let debts: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_debts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((debts, common::balance(&pool, a).await), (1, 0));
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn dispute_webhook_is_a_documented_501_placeholder(pool: PgPool) {
    let (s, _, b) = call(
        &pool,
        "POST",
        "/webhooks/payments/ecpay/dispute",
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (
            StatusCode::NOT_IMPLEMENTED,
            Some("DISPUTE_WEBHOOK_NOT_AVAILABLE")
        )
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn dispute_transitions_touch_updated_at(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, .. } = common::user(&pool, "甲").await;
    let topup = topup_credit(&pool, a, 100).await;
    for action in ["open", "won", "open", "lost"] {
        sqlx::query("UPDATE topups SET updated_at = now() - interval '1 day' WHERE id = $1")
            .bind(topup)
            .execute(&pool)
            .await
            .unwrap();
        let (s, _, b) = dispute(
            &pool,
            &staff.token,
            topup,
            Uuid::new_v4(),
            json!({ "action": action }),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{action}: {b}");
        let fresh: bool = sqlx::query_scalar(
            "SELECT updated_at > now() - interval '1 minute' FROM topups WHERE id = $1",
        )
        .bind(topup)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(fresh, "{action} 應更新 updated_at");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topups_search_transaction_numbers(pool: PgPool) {
    let st = common::staff(&pool, "營運").await;
    let u = common::user(&pool, "付款人").await;
    let id = topup_credit(&pool, u.id, 100).await;
    topup_credit(&pool, u.id, 100).await;
    sqlx::query("UPDATE topups SET merchant_trade_no='Merchant_123', provider_txn_id='Provider%456' WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    for q in ["merchant_123", "provider%25456", "%25"] {
        let (s, r) = topup_get(&pool, &st.token, &format!("?q={q}")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(topup_ids(&r), [id.to_string()]);
    }
    let (_, r) = topup_get(&pool, &st.token, "?q=merchant_123&status=failed").await;
    assert!(topup_ids(&r).is_empty());
}
