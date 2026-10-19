//! 點數批次（D55）：扣點順序、跨批次拆帳、退回原批次、對帳、回填、topup 批次防重複入帳。
mod common;
use axum::http::StatusCode;
use common::{
    assert_all_reconciled, crowdfund_item, grant, lots, pledge, seed_lot, test_app, topup_credit,
    TestUser,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::points;

/// 已啟用清單 + 加密收件地址（達標時 capture_and_order 需要）
async fn list_with_shipping(pool: &PgPool, owner: Uuid) -> Uuid {
    let w = common::wishlist(pool, owner, common::ListOpts::default())
        .await
        .id;
    let sealed = |s: &str| wishly_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4)")
        .bind(w).bind(sealed("王小米")).bind(sealed("0912345678")).bind(sealed("台北市")).execute(pool).await.unwrap();
    w
}

fn sources(l: &[(String, i64)]) -> Vec<(&str, i64)> {
    l.iter().map(|(s, n)| (s.as_str(), *n)).collect()
}

async fn withdraw(pool: &PgPool, token: &str, cid: &str) {
    let (s, _, b) = common::call(
        pool,
        "DELETE",
        &format!("/contributions/{cid}"),
        &common::ck(token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

/// 五個批次：topup 100、adjustment 50、gift_card 30、bonus（10 天到期）20、bonus（5 天到期）10
async fn mixed_wallet(pool: &PgPool, user: Uuid) {
    topup_credit(pool, user, 100).await;
    grant(pool, user, 50).await;
    seed_lot(pool, user, "gift_card", 30, None).await;
    seed_lot(pool, user, "reward_bonus", 20, Some("10 days")).await;
    seed_lot(pool, user, "reward_bonus", 10, Some("5 days")).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_deducts_lots_in_order_splits_ledger_and_withdraw_restores_original_lots(
    pool: PgPool,
) {
    let app = test_app(&pool);
    let owner = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    mixed_wallet(&pool, a).await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    assert_all_reconciled(&pool).await;

    // 認捐 100：最早到期 bonus(10) → bonus(20) → gift(30) → adjustment(40)
    let (s, _, r) = pledge(&app, item, &ta, 100).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["wallet"]["balance"], 110);
    let c1 = r["contribution"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        sources(&lots(&pool, a).await),
        [
            ("topup", 100),
            ("adjustment", 10),
            ("gift_card", 0),
            ("reward_bonus", 0),
            ("reward_bonus", 0)
        ],
        "扣點順序必須是 bonus(最早到期) → gift_card → adjustment → topup"
    );
    // 一筆 pledge 拆成四列 ledger（各帶 lot_id），balance_after 逐筆遞減到最終餘額
    let rows: Vec<(i64, i64, String)> = sqlx::query_as("SELECT delta, balance_after, l.source::text FROM point_ledger g JOIN point_lots l ON l.id = g.lot_id WHERE g.entry_type = 'pledge' ORDER BY g.seq")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        rows,
        [
            (-10, 200, "reward_bonus".into()),
            (-20, 180, "reward_bonus".into()),
            (-30, 150, "gift_card".into()),
            (-40, 110, "adjustment".into())
        ]
    );
    let allocs: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(points), 0)::bigint FROM contribution_lot_allocations WHERE contribution_id = $1::uuid",
    )
    .bind(&c1)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(allocs, 100, "pledge 必須寫入 allocations");
    assert_all_reconciled(&pool).await;

    // 再認捐 70：adjustment 剩 10，其餘 60 由 topup 扣
    let (s, _, r) = pledge(&app, item, &ta, 70).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let c2 = r["contribution"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        sources(&lots(&pool, a).await)[..2],
        [("topup", 40), ("adjustment", 0)]
    );
    assert_all_reconciled(&pool).await;

    // 撤回第一筆：逆序退回原批次（adjustment 40、gift 30、bonus 各自原額）
    withdraw(&pool, &ta, &c1).await;
    assert_eq!(
        sources(&lots(&pool, a).await),
        [
            ("topup", 40),
            ("adjustment", 40),
            ("gift_card", 30),
            ("reward_bonus", 20),
            ("reward_bonus", 10)
        ],
        "撤回必須退回原批次"
    );
    assert_all_reconciled(&pool).await;
    withdraw(&pool, &ta, &c2).await;
    assert_eq!(
        sources(&lots(&pool, a).await),
        [
            ("topup", 100),
            ("adjustment", 50),
            ("gift_card", 30),
            ("reward_bonus", 20),
            ("reward_bonus", 10)
        ]
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn partial_refund_and_failed_release_return_to_original_lots_topup_first(pool: PgPool) {
    let app = test_app(&pool);
    let owner = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 40).await;
    grant(&pool, a, 60).await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    let (_, _, r) = pledge(&app, item, &ta, 100).await;
    let cid: Uuid = r["contribution"]["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 0), ("adjustment", 0)]
    );
    sqlx::query("UPDATE contributions SET status = 'captured', captured_at = now() WHERE id = $1")
        .bind(cid)
        .execute(&pool)
        .await
        .unwrap();

    // 差額退回 50：逆序（topup → adjustment）→ topup 40、adjustment 10
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(points::refund_partial(&mut tx, item, 50).await.unwrap(), 50);
    tx.commit().await.unwrap();
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 40), ("adjustment", 10)],
        "差額退回必須先回補可退現金的 topup 批次"
    );
    // 購買失敗：捕獲中尚未退回的 50 點整筆退回，只回到仍有分配餘額的 adjustment
    let mut tx = pool.begin().await.unwrap();
    let rel = points::release(&mut tx, &[cid], &["captured"])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(rel[0].back, 50);
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 40), ("adjustment", 60)]
    );
    sqlx::query("UPDATE wishlist_items SET pledged_points = 0 WHERE id = $1")
        .bind(item)
        .execute(&pool)
        .await
        .unwrap();
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocation_moves_allocations_so_release_still_returns_original_lots(pool: PgPool) {
    let app = test_app(&pool);
    let owner = common::user(&pool, "小米").await;
    let TestUser { id: a, token: ta } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 120).await;
    grant(&pool, a, 80).await;
    let wl = list_with_shipping(&pool, owner.id).await;
    let (src, dst) = (
        crowdfund_item(&pool, wl, 500).await,
        crowdfund_item(&pool, wl, 200).await,
    );
    let (_, _, r) = pledge(&app, src, &ta, 200).await;
    let old: Uuid = r["contribution"]["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day', funding_deadline = now() - interval '1 day' WHERE id = $1").bind(src).execute(&pool).await.unwrap();
    let (s, _, r) = common::call(
        &pool,
        "POST",
        &format!("/contributions/{old}/reallocate"),
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "target_item_id": dst })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let new: Uuid = r["contribution"]["id"].as_str().unwrap().parse().unwrap();
    let n = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM contribution_lot_allocations WHERE contribution_id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!((n(old).await, n(new).await), (0, 2), "分配必須跟著新認捐");
    assert_all_reconciled(&pool).await;

    let mut tx = pool.begin().await.unwrap();
    points::release(&mut tx, &[new], &["captured"])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 120), ("adjustment", 80)]
    );
    sqlx::query(
        "UPDATE wishlist_items SET pledged_points = 0, funding_status = 'open' WHERE id = $1",
    )
    .bind(dst)
    .execute(&pool)
    .await
    .unwrap();
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn negative_adjustment_spans_lots_and_insufficient_changes_nothing(pool: PgPool) {
    let staff = common::staff(&pool, "營運").await;
    let TestUser { id: a, .. } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 30).await;
    grant(&pool, a, 20).await;
    let adjust = |pts: i64| {
        let (pool, token) = (pool.clone(), staff.token.clone());
        async move {
            common::call(
                &pool,
                "POST",
                "/admin/wallets/grants",
                &[common::ck(&token), common::idem(Uuid::new_v4())].concat(),
                Some(json!({ "user_id": a, "points": pts, "reason": "更正" })),
            )
            .await
        }
    };
    let (s, _, b) = adjust(-40).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 10), ("adjustment", 0)],
        "負數 adjustment 先扣 adjustment 再扣 topup"
    );
    let (s, _, b) = adjust(-11).await;
    assert_eq!(
        (s, b["code"].as_str(), b["balance"].as_i64()),
        (StatusCode::CONFLICT, Some("INSUFFICIENT_POINTS"), Some(10))
    );
    assert_eq!(
        sources(&lots(&pool, a).await),
        [("topup", 10), ("adjustment", 0)]
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topup_creates_cash_refundable_lot_and_duplicate_credit_is_blocked(pool: PgPool) {
    let TestUser { id: a, .. } = common::user(&pool, "甲").await;
    let topup = topup_credit(&pool, a, 100).await;
    grant(&pool, a, 5).await;
    let l: Vec<(String, bool, bool, i64)> = sqlx::query_as("SELECT source::text, cash_refundable, expires_at IS NULL, original_points FROM point_lots ORDER BY created_at, id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        l,
        [
            ("topup".into(), true, true, 100),
            ("adjustment".into(), false, true, 5)
        ]
    );
    // 同一筆 topup 再入帳 → point_lots_source_uk 擋下，整筆 rollback
    let mut tx = pool.begin().await.unwrap();
    let wallet = points::ensure_wallet(&mut tx, a).await.unwrap();
    let dup = points::post(
        &mut tx,
        wallet,
        100,
        points::Entry {
            kind: "topup",
            ref_type: "topup",
            ref_id: Some(topup),
            note: None,
            actor: None,
        },
    )
    .await;
    assert!(dup.is_err(), "重複入帳同一筆 topup 必須被唯一索引擋下");
    tx.rollback().await.unwrap();
    // 批次層的唯一鍵本身也擋得住（不依賴 ledger 的 topup 唯一索引）
    let wallet: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(a)
        .fetch_one(&pool)
        .await
        .unwrap();
    let twice = sqlx::query("INSERT INTO point_lots (wallet_id, source, source_ref_id, original_points, remaining_points, cash_refundable) VALUES ($1, 'topup', $2, 1, 1, true)")
        .bind(wallet).bind(topup).execute(&pool).await;
    assert!(
        twice.is_err(),
        "point_lots_source_uk must reject a second topup lot for the same topup"
    );
    assert_eq!(common::balance(&pool, a).await, 105);
    assert_all_reconciled(&pool).await;

    // GET /wallet 批次摘要
    let (s, _, b) = common::call(
        &pool,
        "GET",
        "/wallet",
        &common::ck(&common::session(&pool, a).await),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["wallet"]["balance"], 105);
    assert_eq!(b["wallet"]["cash_refundable_balance"], 100);
    let lots: &Vec<Value> = b["wallet"]["lots"].as_array().unwrap();
    assert_eq!(lots.len(), 2);
    assert_eq!(lots[0]["source"], "topup");
}

// ---------- 對帳函式本身：各檢查獨立擋得住漂移（供 mutate.py 的 lot-* mutant 使用）----------

async fn drift_fixture(pool: &PgPool) -> Uuid {
    let TestUser { id: a, .. } = common::user(pool, "甲").await;
    topup_credit(pool, a, 100).await;
    grant(pool, a, 50).await;
    a
}
async fn must_panic(pool: &PgPool, what: &str) {
    let check = pool.clone();
    assert!(
        tokio::spawn(async move { common::assert_points_reconciled(&check).await })
            .await
            .expect_err(what)
            .is_panic()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn lot_balance_reconciliation_detects_drift(pool: PgPool) {
    let a = drift_fixture(&pool).await;
    common::assert_points_reconciled(&pool).await;
    // balance 與 ledger 同步多 1（只有 Σ lots ≠ balance 這個檢查看得到）
    sqlx::query("UPDATE point_wallets SET balance = balance + 1 WHERE user_id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO point_ledger (wallet_id, delta, balance_after, entry_type, ref_type) SELECT id, 1, balance, 'adjustment', 'manual' FROM point_wallets WHERE user_id = $1").bind(a).execute(&pool).await.unwrap();
    must_panic(&pool, "lot reconciliation accepted a drifted balance").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn lot_ledger_reconciliation_detects_drift(pool: PgPool) {
    let a = drift_fixture(&pool).await;
    // 批次之間搬 1 點：Σ lots 不變，但個別批次與其 ledger 不符
    sqlx::query(
        "UPDATE point_lots SET remaining_points = remaining_points - 1 WHERE source = 'topup'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE point_lots SET remaining_points = remaining_points + 1, original_points = original_points + 1 WHERE source = 'adjustment'").execute(&pool).await.unwrap();
    let _ = a;
    must_panic(&pool, "lot reconciliation accepted a drifted lot").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn allocation_reconciliation_detects_drift(pool: PgPool) {
    let app = test_app(&pool);
    let owner = common::user(&pool, "小米").await;
    let a = drift_fixture(&pool).await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    let tok = common::session(&pool, a).await;
    assert_eq!(pledge(&app, item, &tok, 20).await.0, StatusCode::CREATED);
    common::assert_points_reconciled(&pool).await;
    sqlx::query("UPDATE contribution_lot_allocations SET refunded_points = 1")
        .execute(&pool)
        .await
        .unwrap();
    must_panic(&pool, "allocation reconciliation accepted a drift").await;
}

// ---------- 回填 ----------

#[sqlx::test(migrations = "../../db/migrations")]
async fn backfill_gives_legacy_balances_and_open_pledges_a_lot(pool: PgPool) {
    let owner = common::user(&pool, "小米").await;
    let wl = common::wishlist(&pool, owner.id, Default::default())
        .await
        .id;
    let item = crowdfund_item(&pool, wl, 1000).await;
    // 舊資料：沒有批次的錢包。A 餘額 70 + 認捐中 30；B 餘額 0 + 已捕獲 50（已退 20）；C 空錢包；D 餘額 25
    let mut w = std::collections::HashMap::new();
    for (name, bal) in [("A", 70i64), ("B", 0), ("C", 0), ("D", 25)] {
        let u = common::user(&pool, name).await;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO point_wallets (user_id, balance) VALUES ($1, $2) RETURNING id",
        )
        .bind(u.id)
        .bind(bal)
        .fetch_one(&pool)
        .await
        .unwrap();
        w.insert(name, (u.id, id));
    }
    let mut cids = std::collections::HashMap::new();
    for (name, status, pts, refunded) in [
        ("A", "pledged", 30i64, 0i64),
        ("B", "captured", 50, 20),
        ("A", "released", 99, 99),
    ] {
        let (uid, wid) = w[name];
        let id: Uuid = sqlx::query_scalar("INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points, refunded_points, status, captured_at, released_at)
            VALUES ($1, $2, $3, $4, 'x', $5, $6, $7::contribution_status, now(), now()) RETURNING id")
            .bind(item).bind(wl).bind(uid).bind(wid).bind(pts).bind(refunded).bind(status).fetch_one(&pool).await.unwrap();
        cids.insert((name, status), id);
    }
    // 重跑 0022 的回填段（migration 已在空資料上跑過，此時表內還沒有批次）
    let sql = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../db/migrations/0022_point_lots.sql"),
    )
    .unwrap();
    let backfill = &sql[sql.find("-- ---------- 回填").unwrap()..];
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM point_lots")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::raw_sql(backfill).execute(&pool).await.unwrap();

    let got: Vec<(Uuid, i64, i64, bool)> = sqlx::query_as(
        "SELECT wallet_id, original_points, remaining_points, cash_refundable FROM point_lots ORDER BY original_points",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut want = vec![
        (w["A"].1, 100, 70, false),
        (w["B"].1, 30, 0, false),
        (w["D"].1, 25, 25, false),
    ];
    want.sort_by_key(|x| x.1);
    assert_eq!(
        got, want,
        "C（空錢包）不建批次；original = balance + 認捐中點數"
    );
    let wallet_bad: Vec<Uuid> = sqlx::query_scalar("SELECT w.id FROM point_wallets w WHERE w.balance <> COALESCE((SELECT sum(remaining_points) FROM point_lots l WHERE l.wallet_id = w.id), 0)").fetch_all(&pool).await.unwrap();
    assert!(wallet_bad.is_empty(), "回填後 Σ lots 必須等於 balance");
    let allocs: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT contribution_id, points, refunded_points FROM contribution_lot_allocations ORDER BY points",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        allocs,
        [
            (cids[&("A", "pledged")], 30, 0),
            (cids[&("B", "captured")], 50, 20)
        ],
        "只有 pledged / captured 認捐有分配"
    );

    // 回填後 release / refund 能退回：A 的 pledged 30 → 餘額 100
    let mut tx = pool.begin().await.unwrap();
    points::release(&mut tx, &[cids[&("A", "pledged")]], &["pledged"])
        .await
        .unwrap();
    points::release(&mut tx, &[cids[&("B", "captured")]], &["captured"])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let after: Vec<(i64, i64)> = sqlx::query_as("SELECT w.balance, (SELECT sum(remaining_points) FROM point_lots l WHERE l.wallet_id = w.id)::bigint FROM point_wallets w WHERE w.id = ANY($1) ORDER BY w.balance")
        .bind(vec![w["A"].1, w["B"].1]).fetch_all(&pool).await.unwrap();
    assert_eq!(after, [(30, 30), (100, 100)]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn ledger_ref_unique_is_per_lot(pool: PgPool) {
    let TestUser { id: a, .. } = common::user(&pool, "甲").await;
    topup_credit(&pool, a, 10).await;
    grant(&pool, a, 10).await;
    let (wallet, lot_a, lot_b): (Uuid, Uuid, Uuid) = sqlx::query_as("SELECT w.id, (SELECT id FROM point_lots WHERE wallet_id = w.id AND source = 'topup'), (SELECT id FROM point_lots WHERE wallet_id = w.id AND source = 'adjustment') FROM point_wallets w WHERE w.user_id = $1")
        .bind(a).fetch_one(&pool).await.unwrap();
    let cid = Uuid::new_v4();
    let ins = |lot: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO point_ledger (wallet_id, lot_id, delta, balance_after, entry_type, ref_type, ref_id) VALUES ($1, $2, -1, 0, 'pledge', 'contribution', $3)")
                .bind(wallet).bind(lot).bind(cid).execute(&pool).await
        }
    };
    assert!(ins(lot_a).await.is_ok());
    assert!(
        ins(lot_b).await.is_ok(),
        "point_ledger_ref_uk must allow one pledge split across lots"
    );
    assert!(
        ins(lot_a).await.is_err(),
        "point_ledger_ref_uk must reject a duplicate pledge row for the same lot"
    );
}
