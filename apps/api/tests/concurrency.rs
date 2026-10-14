//! Races are staged with PostgreSQL row locks. No sleeps: pg_stat_activity proves
//! both requests have reached the conflicting statement before the gate opens.
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use common::{assert_claims_reconciled, call};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use tokio::{sync::Semaphore, task::JoinHandle};

use uuid::Uuid;
use wishsync_api::{app, points, AppState};

// SQLx integration-test pools share a global 20-connection semaphore. Each
// gated race needs up to 5 connections; leave capacity for fixture migrations.
static RACE_SLOTS: Semaphore = Semaphore::const_new(2);

type Resp = (StatusCode, axum::http::HeaderMap, Value);
fn request(
    method: &str,
    path: &str,
    token: Option<&str>,
    key: Option<Uuid>,
    body: Value,
) -> Request<Body> {
    let mut headers = token.map(common::ck).unwrap_or_default();
    if let Some(key) = key {
        headers.push(("idempotency-key", key.to_string()));
    }
    common::request(method, &format!("/api/v1{path}"), &headers, Some(body))
}
fn send(app: &Router, req: Request<Body>) -> JoinHandle<Resp> {
    let app = app.clone();
    tokio::spawn(async move { call(&app, req).await })
}
async fn finish(task: JoinHandle<Resp>) -> Resp {
    tokio::time::timeout(Duration::from_secs(15), task)
        .await
        .expect("race did not finish (possible deadlock)")
        .unwrap()
}
async fn gate<'a>(pool: &'a PgPool, table: &str, id: Uuid) -> (Transaction<'a, Postgres>, i32) {
    let mut tx = pool.begin().await.unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let mode = if table == "point_wallets" {
        "NO KEY UPDATE"
    } else {
        "UPDATE"
    };
    sqlx::query(&format!("SELECT id FROM {table} WHERE id=$1 FOR {mode}"))
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    (tx, pid)
}
/// Poll actual database lock state, scoped to this test's isolated database.
/// A timeout is a failure diagnostic, never a scheduling delay.
async fn blocked(pool: &PgPool, blocker: i32, query: &str, count: i64) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database()
                   AND pid<>pg_backend_pid() AND wait_event_type='Lock' AND query LIKE $1
                   AND $2=ANY(pg_blocking_pids(pid))",
            )
            .bind(format!("%{query}%"))
            .bind(blocker)
            .fetch_one(pool)
            .await
            .unwrap();
            if n >= count {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("expected {count} blocked statements containing {query:?}, blocker {blocker}")
    });
}
async fn waiting(pool: &PgPool, query: &str, count: i64) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE $1")
                .bind(format!("%{query}%")).fetch_one(pool).await.unwrap();
            if n >= count { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap_or_else(|_| panic!("expected {count} blocked statements containing {query:?}"));
}
async fn list(pool: &PgPool, owner: Uuid) -> Uuid {
    let id = sqlx::query_scalar("INSERT INTO wishlists(owner_id,type,status,slug,title) VALUES($1,'registry','active',$2,'race') RETURNING id")
        .bind(owner).bind(Uuid::new_v4().simple().to_string()[..10].to_string()).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO shipping_addresses(wishlist_id,recipient_name_enc,phone_enc,address_enc) VALUES($1,$2,$2,$2)")
        .bind(id).bind(wishsync_api::sealed::seal(b"test address")).execute(pool).await.unwrap();
    id
}
async fn quantity(pool: &PgPool, wl: Uuid, qty: i32) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed) VALUES($1,'quantity',$2) RETURNING id")
        .bind(wl).bind(qty).fetch_one(pool).await.unwrap()
}
async fn funding(pool: &PgPool, wl: Uuid, target: i64) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,target_points,funding_status,funding_deadline,fulfillment_type) VALUES($1,'funding',1,'crowdfund',$2,'open',now()+interval '7 days','concierge') RETURNING id")
        .bind(wl).bind(target).fetch_one(pool).await.unwrap()
}
async fn grant(pool: &PgPool, user: Uuid) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let wallet = points::ensure_wallet(&mut tx, user).await.unwrap();
    points::post(
        &mut tx,
        wallet,
        1000,
        points::Entry {
            kind: "grant",
            ref_type: "manual",
            ref_id: None,
            note: Some("race fixture"),
            actor: None,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    wallet
}
async fn pledge(app: &Router, item: Uuid, token: &str, amount: i64) -> Uuid {
    let (status, _, body) = call(
        app,
        request(
            "POST",
            &format!("/items/{item}/contributions"),
            Some(token),
            Some(Uuid::new_v4()),
            json!({"points":amount}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["contribution"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}
async fn balance(pool: &PgPool, user: Uuid) -> i64 {
    sqlx::query_scalar("SELECT balance FROM point_wallets WHERE user_id=$1")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn item_state(pool: &PgPool, id: Uuid) -> (i64, String) {
    sqlx::query_as("SELECT pledged_points,funding_status::text FROM wishlist_items WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn reconciled(pool: &PgPool) {
    assert_claims_reconciled(pool).await;
    let wallets: Vec<(Uuid,i64,i64)> = sqlx::query_as("SELECT w.id,w.balance,coalesce(sum(l.delta),0)::bigint FROM point_wallets w LEFT JOIN point_ledger l ON l.wallet_id=w.id GROUP BY w.id HAVING w.balance<>coalesce(sum(l.delta),0)").fetch_all(pool).await.unwrap();
    assert!(wallets.is_empty(), "wallet/ledger drift: {wallets:?}");
    let items: Vec<(Uuid,i64,i64)> = sqlx::query_as("SELECT i.id,i.pledged_points,coalesce(sum(c.points) FILTER(WHERE c.status IN ('pledged','captured')),0)::bigint FROM wishlist_items i LEFT JOIN contributions c ON c.item_id=i.id WHERE i.funding_mode='crowdfund' GROUP BY i.id HAVING i.pledged_points<>coalesce(sum(c.points) FILTER(WHERE c.status IN ('pledged','captured')),0)").fetch_all(pool).await.unwrap();
    assert!(items.is_empty(), "item/contribution drift: {items:?}");
    let bad: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlist_items i WHERE i.funding_status='funded' AND (i.pledged_points<>i.target_points OR NOT EXISTS(SELECT 1 FROM purchase_orders o WHERE o.item_id=i.id) OR EXISTS(SELECT 1 FROM contributions c WHERE c.item_id=i.id AND c.status='pledged'))").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0, "funded item must be fully captured with an order");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn same_key_claim_requests_overlap_and_claim_once(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let it = quantity(&pool, list(&pool, owner).await, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let key = Uuid::new_v4();
    let req = || {
        request(
            "POST",
            &format!("/items/{it}/claims"),
            None,
            Some(key),
            json!({"qty":2,"display_name":"guest"}),
        )
    };
    let (gate, pid) = gate(&pool, "wishlist_items", it).await;
    let a = send(&app, req());
    blocked(&pool, pid, "SET qty_claimed", 1).await;
    let b = send(&app, req());
    waiting(&pool, "INSERT INTO idempotency_keys", 1).await;
    gate.commit().await.unwrap();
    let (sa, _, ba) = finish(a).await;
    let (sb, h, bb) = finish(b).await;
    assert_eq!(
        (sa, sb),
        (StatusCode::CREATED, StatusCode::CREATED),
        "{ba} {bb}"
    );
    assert_eq!(h["idempotency-replayed"], "true");
    assert_eq!(ba["claim"], bb["claim"]);
    let state: (i32,i64,i64) = sqlx::query_as("SELECT qty_claimed,(SELECT count(*) FROM claims),(SELECT count(*) FROM guests) FROM wishlist_items WHERE id=$1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(state, (2, 1, 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM idempotency_keys WHERE response_status=201"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn same_key_pledge_requests_overlap_and_debit_once(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (donor, token) = common::user(&pool, "donor").await;
    grant(&pool, donor).await;
    let it = funding(&pool, list(&pool, owner).await, 500).await;
    let app = app(AppState { pool: pool.clone() });
    let key = Uuid::new_v4();
    let req = || {
        request(
            "POST",
            &format!("/items/{it}/contributions"),
            Some(&token),
            Some(key),
            json!({"points":200}),
        )
    };
    let (gate, pid) = gate(&pool, "wishlist_items", it).await;
    let a = send(&app, req());
    blocked(&pool, pid, "SET pledged_points", 1).await;
    let b = send(&app, req());
    waiting(&pool, "INSERT INTO idempotency_keys", 1).await;
    gate.commit().await.unwrap();
    let (sa, _, ba) = finish(a).await;
    let (sb, h, bb) = finish(b).await;
    assert_eq!(
        (sa, sb),
        (StatusCode::CREATED, StatusCode::CREATED),
        "{ba} {bb}"
    );
    assert_eq!(h["idempotency-replayed"], "true");
    assert_eq!(ba, bb);
    assert_eq!(balance(&pool, donor).await, 800);
    assert_eq!(item_state(&pool, it).await, (200, "open".into()));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM contributions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM point_ledger WHERE entry_type='pledge'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mixed_quantity_claims_report_remaining_and_never_oversell(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (_, ta) = common::user(&pool, "a").await;
    let (_, tb) = common::user(&pool, "b").await;
    let (_, tc) = common::user(&pool, "c").await;
    let app = app(AppState { pool: pool.clone() });
    for quantities in [[2, 3, 3], [2, 2, 2]] {
        let it = quantity(&pool, list(&pool, owner).await, 5).await;
        let req = |token: &str, qty| {
            request(
                "POST",
                &format!("/items/{it}/claims"),
                Some(token),
                Some(Uuid::new_v4()),
                json!({"qty":qty}),
            )
        };
        let (gate, pid) = gate(&pool, "wishlist_items", it).await;
        let a = send(&app, req(&ta, quantities[0]));
        blocked(&pool, pid, "SET qty_claimed", 1).await;
        let b = send(&app, req(&tb, quantities[1]));
        waiting(&pool, "SET qty_claimed", 2).await;
        let c = send(&app, req(&tc, quantities[2]));
        waiting(&pool, "SET qty_claimed", 3).await;
        gate.commit().await.unwrap();
        let ra = finish(a).await;
        let rb = finish(b).await;
        let rc = finish(c).await;
        let responses = [
            (quantities[0], ra),
            (quantities[1], rb),
            (quantities[2], rc),
        ];
        let successful: Vec<_> = responses
            .iter()
            .filter(|(_, r)| r.0 == StatusCode::CREATED)
            .collect();
        assert_eq!(successful.len(), 2, "{responses:?}");
        let claimed: i32 = successful.iter().map(|(qty, _)| qty).sum();
        assert_eq!(claimed, if quantities[1] == 3 { 5 } else { 4 });
        for (qty, r) in &responses {
            if r.0 == StatusCode::CREATED {
                assert_eq!(r.2["claim"]["qty"], *qty);
            } else {
                assert_eq!(r.0, StatusCode::CONFLICT, "{}", r.2);
                assert_eq!(r.2["code"], "ITEM_FULLY_CLAIMED");
                let remaining = r.2["remaining"].as_i64().unwrap();
                assert!(remaining < *qty as i64 && remaining >= 0);
                // A retrying UPDATE can reorder waiters. The error's remaining
                // must match a committed success snapshot, not necessarily the
                // final total after another in-flight success also commits.
                assert!(
                    successful
                        .iter()
                        .any(|(_, s)| s.2["item"]["qty_claimed"] == 5 - remaining),
                    "{r:?}"
                );
                if quantities[1] == 2 {
                    assert_eq!(remaining, 1);
                }
            }
        }
        assert_eq!(
            sqlx::query_scalar::<_, i32>("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
                .bind(it)
                .fetch_one(&pool)
                .await
                .unwrap(),
            claimed
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM claims WHERE item_id=$1")
                .bind(it)
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        reconciled(&pool).await;
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_vs_final_pledge_serializes_both_orders(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (a, ta) = common::user(&pool, "a").await;
    let (b, tb) = common::user(&pool, "b").await;
    grant(&pool, a).await;
    grant(&pool, b).await;
    let wl = list(&pool, owner).await;
    let app = app(AppState { pool: pool.clone() });
    for pledge_first in [false, true] {
        let it = funding(&pool, wl, 500).await;
        let cid = pledge(&app, it, &ta, 300).await;
        let (gate, pid) = gate(&pool, "wishlist_items", it).await;
        let withdrawal = || {
            request(
                "DELETE",
                &format!("/contributions/{cid}"),
                Some(&ta),
                None,
                Value::Null,
            )
        };
        let donation = || {
            request(
                "POST",
                &format!("/items/{it}/contributions"),
                Some(&tb),
                Some(Uuid::new_v4()),
                json!({"points":200}),
            )
        };
        let (w, p) = if pledge_first {
            let p = send(&app, donation());
            blocked(&pool, pid, "SET pledged_points", 1).await;
            let w = send(&app, withdrawal());
            waiting(&pool, "SELECT funding_status = 'open'", 1).await;
            (w, p)
        } else {
            let w = send(&app, withdrawal());
            blocked(&pool, pid, "SELECT funding_status = 'open'", 1).await;
            let p = send(&app, donation());
            waiting(&pool, "SET pledged_points", 1).await;
            (w, p)
        };
        gate.commit().await.unwrap();
        let wr = finish(w).await;
        let pr = finish(p).await;
        assert_eq!(pr.0, StatusCode::CREATED, "{}", pr.2);
        let status: String =
            sqlx::query_scalar("SELECT status::text FROM contributions WHERE id=$1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        let orders: i64 =
            sqlx::query_scalar("SELECT count(*) FROM purchase_orders WHERE item_id=$1")
                .bind(it)
                .fetch_one(&pool)
                .await
                .unwrap();
        if pledge_first {
            assert_eq!(wr.0, StatusCode::CONFLICT, "{}", wr.2);
            assert_eq!(wr.2["code"], "CONTRIBUTION_LOCKED");
            assert_eq!(status, "captured");
            assert_eq!(orders, 1);
            assert_eq!(item_state(&pool, it).await, (500, "funded".into()));
        } else {
            assert_eq!(wr.0, StatusCode::OK, "{}", wr.2);
            assert_eq!(status, "released");
            assert_eq!(orders, 0);
            assert_eq!(item_state(&pool, it).await, (200, "open".into()));
        }
        reconciled(&pool).await;
    }
    assert_eq!(balance(&pool, a).await, 700);
    assert_eq!(balance(&pool, b).await, 600);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_vs_tick_refunds_exactly_once(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (donor, token) = common::user(&pool, "donor").await;
    let wallet = grant(&pool, donor).await;
    let wl = list(&pool, owner).await;
    let app = app(AppState { pool: pool.clone() });
    // Both expiry transition and the automatic-refund phase race withdrawal.
    for auto_refund in [false, true] {
        let it = funding(&pool, wl, 500).await;
        let cid = pledge(&app, it, &token, 200).await;
        if auto_refund {
            sqlx::query("UPDATE wishlist_items SET funding_status='expired',expired_at=now()-interval '8 days' WHERE id=$1").bind(it).execute(&pool).await.unwrap();
        } else {
            sqlx::query(
                "UPDATE wishlist_items SET funding_deadline=now()-interval '1 hour' WHERE id=$1",
            )
            .bind(it)
            .execute(&pool)
            .await
            .unwrap();
        }
        let (gate, pid) = gate(&pool, "point_wallets", wallet).await;
        let w = send(
            &app,
            request(
                "DELETE",
                &format!("/contributions/{cid}"),
                Some(&token),
                None,
                Value::Null,
            ),
        );
        blocked(&pool, pid, "SELECT id FROM point_wallets", 1).await;
        let p = pool.clone();
        let mut tick = Some(tokio::spawn(async move {
            wishsync_api::contributions::tick_funding(&p).await
        }));
        if auto_refund {
            waiting(&pool, "SELECT id FROM point_wallets", 2).await;
        }
        // With open -> expired, tick need not wait for the wallet, and can finish
        // the transition while withdrawal is demonstrably in flight.
        if !auto_refund {
            assert_eq!(tick.take().unwrap().await.unwrap().unwrap(), (1, 0));
        }
        gate.commit().await.unwrap();
        let wr = finish(w).await;
        if auto_refund {
            let (_, released) = tick.take().unwrap().await.unwrap().unwrap();
            assert_eq!(released, 1);
            assert!(
                wr.0 == StatusCode::OK || wr.0 == StatusCode::CONFLICT,
                "{}",
                wr.2
            );
        } else {
            assert_eq!(wr.0, StatusCode::OK, "{}", wr.2);
        }
        assert_eq!(balance(&pool, donor).await, 1000);
        assert_eq!(item_state(&pool, it).await, (0, "expired".into()));
        let row: (String, i64) =
            sqlx::query_as("SELECT status::text,refunded_points FROM contributions WHERE id=$1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row, ("released".into(), 200));
        let refunds: (i64,i64) = sqlx::query_as("SELECT count(*),sum(delta)::bigint FROM point_ledger WHERE entry_type='release' AND ref_id=$1").bind(cid).fetch_one(&pool).await.unwrap();
        assert_eq!(refunds, (1, 200));
        reconciled(&pool).await;
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_vs_last_pledge_respects_target_cap_in_both_orders(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (a, ta) = common::user(&pool, "a").await;
    let (b, tb) = common::user(&pool, "b").await;
    grant(&pool, a).await;
    grant(&pool, b).await;
    let wl = list(&pool, owner).await;
    let app = app(AppState { pool: pool.clone() });
    for reallocate_first in [false, true] {
        let src = funding(&pool, wl, 500).await;
        let tgt = funding(&pool, wl, 200).await;
        let cid = pledge(&app, src, &ta, 200).await;
        sqlx::query(
            "UPDATE wishlist_items SET funding_status='expired',expired_at=now() WHERE id=$1",
        )
        .bind(src)
        .execute(&pool)
        .await
        .unwrap();
        let (gate, pid) = gate(&pool, "wishlist_items", tgt).await;
        let reallocation = || {
            request(
                "POST",
                &format!("/contributions/{cid}/reallocate"),
                Some(&ta),
                Some(Uuid::new_v4()),
                json!({"target_item_id":tgt}),
            )
        };
        let donation = || {
            request(
                "POST",
                &format!("/items/{tgt}/contributions"),
                Some(&tb),
                Some(Uuid::new_v4()),
                json!({"points":200}),
            )
        };
        let (r, p) = if reallocate_first {
            let r = send(&app, reallocation());
            blocked(&pool, pid, "ORDER BY id FOR UPDATE", 1).await;
            let p = send(&app, donation());
            waiting(&pool, "SET pledged_points", 1).await;
            (r, p)
        } else {
            let p = send(&app, donation());
            blocked(&pool, pid, "SET pledged_points", 1).await;
            let r = send(&app, reallocation());
            waiting(&pool, "ORDER BY id FOR UPDATE", 1).await;
            (r, p)
        };
        gate.commit().await.unwrap();
        let rr = finish(r).await;
        let pr = finish(p).await;
        assert_eq!(
            (rr.0, pr.0),
            if reallocate_first {
                (StatusCode::CREATED, StatusCode::CONFLICT)
            } else {
                (StatusCode::CONFLICT, StatusCode::CREATED)
            },
            "{} {}",
            rr.2,
            pr.2
        );
        assert_eq!(item_state(&pool, tgt).await, (200, "funded".into()));
        assert_eq!(
            item_state(&pool, src).await,
            if reallocate_first {
                (0, "expired".into())
            } else {
                (200, "expired".into())
            }
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM contributions WHERE item_id=$1 AND status='captured'"
            )
            .bind(tgt)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM purchase_orders WHERE item_id=$1")
                .bind(tgt)
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        reconciled(&pool).await;
    }
    assert_eq!(balance(&pool, a).await, 600);
    assert_eq!(balance(&pool, b).await, 800);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn opposite_reallocations_lock_lowest_item_first_without_deadlock(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (a, ta) = common::user(&pool, "a").await;
    let (b, tb) = common::user(&pool, "b").await;
    grant(&pool, a).await;
    grant(&pool, b).await;
    let wl = list(&pool, owner).await;
    let app = app(AppState { pool: pool.clone() });
    let x = funding(&pool, wl, 500).await;
    let y = funding(&pool, wl, 500).await;
    let (low, high) = if x < y { (x, y) } else { (y, x) };
    let ca = pledge(&app, high, &ta, 200).await;
    let cb = pledge(&app, low, &tb, 200).await;
    sqlx::query(
        "UPDATE wishlist_items SET funding_status='expired',expired_at=now() WHERE wishlist_id=$1",
    )
    .bind(wl)
    .execute(&pool)
    .await
    .unwrap();
    // Opposite transfers cannot both be valid: a source must be expired while
    // its target must be open. They still must acquire the two locks in one order.
    let (low_gate, low_pid) = gate(&pool, "wishlist_items", low).await;
    let (high_gate, high_pid) = gate(&pool, "wishlist_items", high).await;
    let r = send(
        &app,
        request(
            "POST",
            &format!("/contributions/{ca}/reallocate"),
            Some(&ta),
            Some(Uuid::new_v4()),
            json!({"target_item_id":low}),
        ),
    );
    blocked(
        &pool,
        low_pid,
        "SELECT id FROM wishlist_items WHERE id IN",
        1,
    )
    .await;
    // The second request is started after high -> low demonstrably waits on low;
    // the unrelated high blocker must not be the first lock being requested.
    let p = send(
        &app,
        request(
            "POST",
            &format!("/contributions/{cb}/reallocate"),
            Some(&tb),
            Some(Uuid::new_v4()),
            json!({"target_item_id":high}),
        ),
    );
    waiting(&pool, "SELECT id FROM wishlist_items WHERE id IN", 2).await;
    let high_waiters: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid))").bind(high_pid).fetch_one(&pool).await.unwrap();
    assert_eq!(
        high_waiters, 0,
        "both directions must acquire lowest item first"
    );
    high_gate.commit().await.unwrap();
    low_gate.commit().await.unwrap();
    let rr = finish(r).await;
    let pr = finish(p).await;
    assert_eq!(
        (rr.0, pr.0),
        (StatusCode::CONFLICT, StatusCode::CONFLICT),
        "{} {}",
        rr.2,
        pr.2
    );
    assert_eq!(item_state(&pool, low).await, (200, "expired".into()));
    assert_eq!(item_state(&pool, high).await, (200, "expired".into()));
    assert_eq!(balance(&pool, a).await, 800);
    assert_eq!(balance(&pool, b).await, 800);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM contributions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_freeze_vs_pledge_checks_status_under_lock(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (_, staff) = common::user_with_staff(&pool, "staff", true).await;
    let (donor, token) = common::user(&pool, "donor").await;
    let wallet = grant(&pool, donor).await;
    let it = funding(&pool, list(&pool, owner).await, 500).await;
    let app = app(AppState { pool: pool.clone() });
    let (gate, pid) = gate(&pool, "point_wallets", wallet).await;
    let freeze = send(
        &app,
        request(
            "PATCH",
            &format!("/admin/wallets/{wallet}"),
            Some(&staff),
            None,
            json!({"status":"frozen","reason":"race"}),
        ),
    );
    blocked(&pool, pid, "SELECT status::text FROM point_wallets", 1).await;
    let p = send(
        &app,
        request(
            "POST",
            &format!("/items/{it}/contributions"),
            Some(&token),
            Some(Uuid::new_v4()),
            json!({"points":200}),
        ),
    );
    waiting(&pool, "point_wallets", 2).await;
    gate.commit().await.unwrap();
    let f = finish(freeze).await;
    let p = finish(p).await;
    assert_eq!(f.0, StatusCode::OK, "{}", f.2);
    assert_eq!(
        balance(&pool, donor).await,
        1000,
        "frozen wallet was debited by the racing pledge"
    );
    assert_eq!(p.0, StatusCode::FORBIDDEN, "{}", p.2);
    assert_eq!(p.2["code"], "WALLET_FROZEN");
    assert_eq!(item_state(&pool, it).await, (0, "open".into()));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM point_wallets WHERE id=$1")
            .bind(wallet)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "frozen"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM contributions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM point_ledger WHERE entry_type='pledge'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn removal_vs_inflight_claim_serializes_and_reconciles(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, owner_token) = common::user(&pool, "owner").await;
    let (_, donor_token) = common::user(&pool, "donor").await;
    let app = app(AppState { pool: pool.clone() });
    for archive in [false, true] {
        for removal_first in [false, true] {
            let wl = list(&pool, owner).await;
            let it = quantity(&pool, wl, 5).await;
            let remove = || {
                request(
                    "DELETE",
                    &if archive {
                        format!("/wishlists/{wl}")
                    } else {
                        format!("/items/{it}?force=true")
                    },
                    Some(&owner_token),
                    None,
                    Value::Null,
                )
            };
            let claim = || {
                request(
                    "POST",
                    &format!("/items/{it}/claims"),
                    Some(&donor_token),
                    Some(Uuid::new_v4()),
                    json!({"qty":2}),
                )
            };
            let (c, r) = if removal_first {
                let (gate, pid) = gate(&pool, "wishlists", wl).await;
                let r = send(&app, remove());
                blocked(
                    &pool,
                    pid,
                    if archive {
                        "UPDATE wishlists SET status='archived'"
                    } else {
                        "SELECT id FROM wishlists"
                    },
                    1,
                )
                .await;
                let c = send(&app, claim());
                waiting(&pool, "FOR SHARE", 1).await;
                gate.commit().await.unwrap();
                (c, r)
            } else {
                let (gate, pid) = gate(&pool, "wishlist_items", it).await;
                let c = send(&app, claim());
                blocked(&pool, pid, "SET qty_claimed", 1).await;
                let r = send(&app, remove());
                waiting(
                    &pool,
                    if archive {
                        "UPDATE wishlists SET status='archived'"
                    } else {
                        "SELECT id FROM wishlists"
                    },
                    1,
                )
                .await;
                gate.commit().await.unwrap();
                (c, r)
            };
            let cr = finish(c).await;
            let rr = finish(r).await;
            assert_eq!(rr.0, StatusCode::NO_CONTENT, "{}", rr.2);
            if removal_first {
                assert!(
                    cr.0 == StatusCode::CONFLICT || cr.0 == StatusCode::NOT_FOUND,
                    "{} {}",
                    cr.0,
                    cr.2
                );
            } else {
                assert_eq!(cr.0, StatusCode::CREATED, "{}", cr.2);
            }
            let (qty, deleted): (i32, bool) = sqlx::query_as(
                "SELECT qty_claimed,deleted_at IS NOT NULL FROM wishlist_items WHERE id=$1",
            )
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(deleted, !archive);
            // Archiving keeps existing claims; force item deletion cancels them.
            assert_eq!(qty, if archive && !removal_first { 2 } else { 0 });
            let active: i64 = sqlx::query_scalar("SELECT count(*) FROM claims WHERE item_id=$1 AND status IN ('reserved','purchased','delivered')").bind(it).fetch_one(&pool).await.unwrap();
            assert_eq!(active, if archive && !removal_first { 1 } else { 0 });
            let status: String =
                sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id=$1")
                    .bind(wl)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(status, if archive { "archived" } else { "active" });
            reconciled(&pool).await;
        }
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn removal_vs_inflight_pledge_serializes_and_refunds(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, owner_token) = common::user(&pool, "owner").await;
    let (donor, donor_token) = common::user(&pool, "donor").await;
    grant(&pool, donor).await;
    let app = app(AppState { pool: pool.clone() });
    for archive in [false, true] {
        for removal_first in [false, true] {
            let wl = list(&pool, owner).await;
            let it = funding(&pool, wl, 500).await;
            let remove = || {
                request(
                    "DELETE",
                    &if archive {
                        format!("/wishlists/{wl}")
                    } else {
                        format!("/items/{it}?force=true")
                    },
                    Some(&owner_token),
                    None,
                    Value::Null,
                )
            };
            let donate = || {
                request(
                    "POST",
                    &format!("/items/{it}/contributions"),
                    Some(&donor_token),
                    Some(Uuid::new_v4()),
                    json!({"points":200}),
                )
            };
            let (p, r) = if removal_first {
                let (gate, pid) = gate(&pool, "wishlists", wl).await;
                let r = send(&app, remove());
                blocked(
                    &pool,
                    pid,
                    if archive {
                        "UPDATE wishlists SET status='archived'"
                    } else {
                        "SELECT id FROM wishlists"
                    },
                    1,
                )
                .await;
                let p = send(&app, donate());
                waiting(&pool, "FOR SHARE", 1).await;
                gate.commit().await.unwrap();
                (p, r)
            } else {
                let (gate, pid) = gate(&pool, "wishlist_items", it).await;
                let p = send(&app, donate());
                blocked(&pool, pid, "SET pledged_points", 1).await;
                let r = send(&app, remove());
                waiting(
                    &pool,
                    if archive {
                        "UPDATE wishlists SET status='archived'"
                    } else {
                        "SELECT id FROM wishlists"
                    },
                    1,
                )
                .await;
                gate.commit().await.unwrap();
                (p, r)
            };
            let pr = finish(p).await;
            let rr = finish(r).await;
            assert_eq!(rr.0, StatusCode::NO_CONTENT, "{}", rr.2);
            if removal_first {
                assert!(
                    pr.0 == StatusCode::CONFLICT || pr.0 == StatusCode::NOT_FOUND,
                    "{} {}",
                    pr.0,
                    pr.2
                );
            } else {
                assert_eq!(pr.0, StatusCode::CREATED, "{}", pr.2);
            }
            assert_eq!(balance(&pool, donor).await, 1000);
            assert_eq!(item_state(&pool, it).await, (0, "open".into()));
            let rows: Vec<(String, i64)> = sqlx::query_as(
                "SELECT status::text,refunded_points FROM contributions WHERE item_id=$1",
            )
            .bind(it)
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(
                rows,
                if removal_first {
                    vec![]
                } else {
                    vec![("released".into(), 200)]
                }
            );
            assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM point_ledger WHERE entry_type='release' AND ref_id IN(SELECT id FROM contributions WHERE item_id=$1)").bind(it).fetch_one(&pool).await.unwrap(),if removal_first {0}else{1});
            reconciled(&pool).await;
        }
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn deleting_quantity_item_preserves_delivered_claim_accounting(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, token) = common::user(&pool, "owner").await;
    let (_, donor) = common::user(&pool, "donor").await;
    let wl = list(&pool, owner).await;
    let it = quantity(&pool, wl, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let (s, _, body) = call(
        &app,
        request(
            "POST",
            &format!("/items/{it}/claims"),
            Some(&donor),
            Some(Uuid::new_v4()),
            json!({"qty":2}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let cid = body["claim"]["id"].as_str().unwrap();
    let (s, _, body) = call(
        &app,
        request(
            "PATCH",
            &format!("/claims/{cid}"),
            Some(&donor),
            None,
            json!({"status":"delivered"}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, _, body) = call(
        &app,
        request(
            "DELETE",
            &format!("/items/{it}?force=true"),
            Some(&token),
            None,
            Value::Null,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn archive_vs_waiting_claim_cannot_accept_after_archive_commit(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, owner_token) = common::user(&pool, "owner").await;
    let (_, donor_token) = common::user(&pool, "donor").await;
    let wl = list(&pool, owner).await;
    let it = quantity(&pool, wl, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let (gate, pid) = gate(&pool, "wishlist_items", it).await;
    let c = send(
        &app,
        request(
            "POST",
            &format!("/items/{it}/claims"),
            Some(&donor_token),
            Some(Uuid::new_v4()),
            json!({"qty":2}),
        ),
    );
    blocked(&pool, pid, "SET qty_claimed", 1).await;
    let a = send(
        &app,
        request(
            "DELETE",
            &format!("/wishlists/{wl}"),
            Some(&owner_token),
            None,
            Value::Null,
        ),
    );
    // Correct locking blocks archive behind the in-flight claim. If a mutation
    // lets archive finish first, the claim must reject rather than use its old
    // UPDATE snapshot. This catches the actual acceptance bug, not only a lock.
    common::wait_until("archive committed or waiting on claim",|| async {
        if a.is_finished() { return true; }
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%UPDATE wishlists SET status=''archived''%')")
            .fetch_one(&pool).await.unwrap()
    }).await;
    let archived_first = a.is_finished();
    gate.commit().await.unwrap();
    let cr = finish(c).await;
    let ar = finish(a).await;
    assert_eq!(ar.0, StatusCode::NO_CONTENT, "{}", ar.2);
    let qty: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    if archived_first {
        assert_eq!(qty, 0, "claim accepted after archive committed");
        assert_eq!(cr.0, StatusCode::CONFLICT, "{}", cr.2);
    } else {
        assert_eq!(qty, 2);
        assert_eq!(cr.0, StatusCode::CREATED, "{}", cr.2);
    }
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn force_delete_vs_claim_update_uses_item_then_claim_lock_order(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, owner_token) = common::user(&pool, "owner").await;
    let (_, donor_token) = common::user(&pool, "donor").await;
    let wl = list(&pool, owner).await;
    let it = quantity(&pool, wl, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let (s, _, b) = call(
        &app,
        request(
            "POST",
            &format!("/items/{it}/claims"),
            Some(&donor_token),
            Some(Uuid::new_v4()),
            json!({"qty":2}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let cid: Uuid = b["claim"]["id"].as_str().unwrap().parse().unwrap();
    let (gate, pid) = gate(&pool, "claims", cid).await;
    let d = send(
        &app,
        request(
            "DELETE",
            &format!("/items/{it}?force=true"),
            Some(&owner_token),
            None,
            Value::Null,
        ),
    );
    blocked(&pool, pid, "UPDATE claims SET status='cancelled'", 1).await;
    let u = send(
        &app,
        request(
            "PATCH",
            &format!("/claims/{cid}"),
            Some(&donor_token),
            None,
            json!({"qty":3}),
        ),
    );
    // Both operations have reached a conflicting row lock. With item -> claim
    // ordering update waits on item; a reversed delete makes it hold item while
    // waiting on claim. Opening the gate then creates a real deadlock.
    waiting(&pool, "FOR UPDATE", 1).await;
    gate.commit().await.unwrap();
    let dr = finish(d).await;
    let ur = finish(u).await;
    assert_eq!(dr.0, StatusCode::NO_CONTENT, "{}", dr.2);
    assert_eq!(ur.0, StatusCode::OK, "{}", ur.2);
    assert_eq!(ur.2["claim"]["status"], "cancelled");
    assert_eq!(ur.2["claim"]["qty"], 2);
    let row: (String, i32) = sqlx::query_as("SELECT status::text,qty FROM claims WHERE id=$1")
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row, ("cancelled".into(), 2));
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_vs_own_claim_never_deadlocks_or_leaves_active_claim(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, token) = common::user(&pool, "owner").await;
    let wl = list(&pool, owner).await;
    let it = quantity(&pool, wl, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let (gate, pid) = gate(&pool, "wishlists", wl).await;
    let c = send(
        &app,
        request(
            "POST",
            &format!("/items/{it}/claims"),
            Some(&token),
            Some(Uuid::new_v4()),
            json!({"qty":2}),
        ),
    );
    blocked(&pool, pid, "FOR SHARE", 1).await;
    let d = send(
        &app,
        request(
            "DELETE",
            "/me",
            Some(&token),
            None,
            json!({"confirm":"DELETE"}),
        ),
    );
    common::wait_until("account deletion waiting on user or wishlist",|| async {
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND (query LIKE '%SELECT is_staff FROM users%' OR query LIKE '%UPDATE wishlists SET status = ''archived''%'))")
            .fetch_one(&pool).await.unwrap()
    }).await;
    gate.commit().await.unwrap();
    let cr = finish(c).await;
    let dr = finish(d).await;
    assert_eq!(cr.0, StatusCode::CREATED, "{}", cr.2);
    assert_eq!(dr.0, StatusCode::OK, "{}", dr.2);
    let row: (bool,String,i32) = sqlx::query_as("SELECT u.deleted_at IS NOT NULL,w.status::text,i.qty_claimed FROM users u JOIN wishlists w ON w.owner_id=u.id JOIN wishlist_items i ON i.wishlist_id=w.id WHERE i.id=$1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(row, (true, "archived".into(), 0));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM claims WHERE user_id=$1 AND status='reserved'"
        )
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_vs_own_pledge_checks_actor_before_wishlist(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, token) = common::user(&pool, "owner").await;
    let wl = list(&pool, owner).await;
    let it = funding(&pool, wl, 500).await;
    let app = app(AppState { pool: pool.clone() });
    let (gate, pid) = gate(&pool, "wishlists", wl).await;
    // No balance: deletion is allowed, while the pledge must roll back its
    // prospective contribution and newly-created wallet for insufficient funds.
    let p = send(
        &app,
        request(
            "POST",
            &format!("/items/{it}/contributions"),
            Some(&token),
            Some(Uuid::new_v4()),
            json!({"points":200}),
        ),
    );
    blocked(&pool, pid, "FOR SHARE", 1).await;
    let d = send(
        &app,
        request(
            "DELETE",
            "/me",
            Some(&token),
            None,
            json!({"confirm":"DELETE"}),
        ),
    );
    common::wait_until("account deletion waiting on user or wishlist",|| async {
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND (query LIKE '%SELECT is_staff FROM users%' OR query LIKE '%UPDATE wishlists SET status = ''archived''%'))")
            .fetch_one(&pool).await.unwrap()
    }).await;
    gate.commit().await.unwrap();
    let pr = finish(p).await;
    let dr = finish(d).await;
    assert_eq!(pr.0, StatusCode::CONFLICT, "{}", pr.2);
    assert_eq!(pr.2["code"], "INSUFFICIENT_POINTS");
    assert_eq!(dr.0, StatusCode::OK, "{}", dr.2);
    assert_eq!(item_state(&pool, it).await, (0, "open".into()));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM users WHERE id=$1 AND deleted_at IS NOT NULL"
        )
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM contributions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM point_wallets")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_vs_guest_claim_locks_owner_before_notification_fk(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, token) = common::user(&pool, "owner").await;
    let wl = list(&pool, owner).await;
    let it = quantity(&pool, wl, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let (gate, pid) = gate(&pool, "wishlists", wl).await;
    let c = send(
        &app,
        request(
            "POST",
            &format!("/items/{it}/claims"),
            None,
            Some(Uuid::new_v4()),
            json!({"qty":2,"display_name":"guest"}),
        ),
    );
    blocked(&pool, pid, "FOR SHARE", 1).await;
    let d = send(
        &app,
        request(
            "DELETE",
            "/me",
            Some(&token),
            None,
            json!({"confirm":"DELETE"}),
        ),
    );
    common::wait_until("owner deletion waiting on user or wishlist",|| async {
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND (query LIKE '%SELECT is_staff FROM users%' OR query LIKE '%UPDATE wishlists SET status = ''archived''%'))")
            .fetch_one(&pool).await.unwrap()
    }).await;
    gate.commit().await.unwrap();
    let cr = finish(c).await;
    let dr = finish(d).await;
    assert_eq!(cr.0, StatusCode::CREATED, "{}", cr.2);
    assert_eq!(dr.0, StatusCode::OK, "{}", dr.2);
    let row: (bool,String,i32) = sqlx::query_as("SELECT u.deleted_at IS NOT NULL,w.status::text,i.qty_claimed FROM users u JOIN wishlists w ON w.owner_id=u.id JOIN wishlist_items i ON i.wishlist_id=w.id WHERE i.id=$1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(row, (true, "archived".into(), 2));
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM claims WHERE item_id=$1 AND status='reserved' AND guest_id IS NOT NULL").bind(it).fetch_one(&pool).await.unwrap(),1);
    reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn deleted_actor_cannot_create_claim_on_another_users_active_list(pool: PgPool) {
    let _slot = RACE_SLOTS.acquire().await.unwrap();
    let (owner, _) = common::user(&pool, "owner").await;
    let (actor, token) = common::user(&pool, "actor").await;
    let it = quantity(&pool, list(&pool, owner).await, 5).await;
    let app = app(AppState { pool: pool.clone() });
    let (gate, pid) = gate(&pool, "users", actor).await;
    let d = send(
        &app,
        request(
            "DELETE",
            "/me",
            Some(&token),
            None,
            json!({"confirm":"DELETE"}),
        ),
    );
    blocked(&pool, pid, "SELECT is_staff FROM users", 1).await;
    // Authentication has succeeded while the actor still exists, but deletion
    // wins before the transaction's actor lock. The locked read must revalidate.
    let c = send(
        &app,
        request(
            "POST",
            &format!("/items/{it}/claims"),
            Some(&token),
            Some(Uuid::new_v4()),
            json!({"qty":2}),
        ),
    );
    waiting(&pool, "SELECT display_name FROM users", 1).await;
    gate.commit().await.unwrap();
    let dr = finish(d).await;
    let cr = finish(c).await;
    assert_eq!(dr.0, StatusCode::OK, "{}", dr.2);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "deleted actor created a new claim"
    );
    assert_eq!(cr.0, StatusCode::UNAUTHORIZED, "{}", cr.2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM claims WHERE user_id=$1")
            .bind(actor)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    reconciled(&pool).await;
}
