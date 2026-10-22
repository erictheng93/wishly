//! Q2：清單結束（closed / archived / closes_at 已過）後，捐助者只能取消，不能標記已購買 / 已送達；擁有者不受限（F8）。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

const CLOSERS: [&str; 3] = [
    "UPDATE wishlists SET status = 'closed', closed_at = now() WHERE id = $1",
    "UPDATE wishlists SET status = 'archived' WHERE id = $1",
    "UPDATE wishlists SET closes_at = now() - interval '1 hour' WHERE id = $1",
];

async fn qty_claimed(pool: &PgPool, item: Uuid) -> i32 {
    sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1")
        .bind(item)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn patch(pool: &PgPool, c: Uuid, h: &[(&str, String)], body: serde_json::Value) -> Resp {
    call(pool, "PATCH", &format!("/claims/{c}"), h, Some(body)).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn donor_can_only_cancel_after_list_closes(pool: PgPool) {
    for close in CLOSERS {
        let o = user(&pool, "o").await;
        let wl = wishlist(&pool, o.id, ListOpts::default()).await;
        let it = item(&pool, wl.id, "奶瓶", 5).await;
        let (t, c) = user_claim(&pool, it, 2).await;
        let (tp, cp) = user_claim(&pool, it, 1).await;
        assert_eq!(
            patch(&pool, cp, &ck(&tp), json!({"status":"purchased"}))
                .await
                .0,
            StatusCode::OK
        );
        sqlx::query(close).bind(wl.id).execute(&pool).await.unwrap();

        for (tok, id, s) in [
            (&t, c, "purchased"),
            (&t, c, "delivered"),
            (&tp, cp, "delivered"),
        ] {
            let (st, _, b) = patch(&pool, id, &ck(tok), json!({ "status": s })).await;
            assert_eq!(st, StatusCode::CONFLICT, "{close} {s}: {b}");
            assert_eq!(b["code"], "WISHLIST_CLOSED");
        }
        // 同狀態 no-op 仍冪等
        let (st, _, b) = patch(&pool, cp, &ck(&tp), json!({"status":"purchased"})).await;
        assert_eq!(st, StatusCode::OK, "{close}: {b}");
        // 備註 / 公開層級仍可改
        let (st, _, b) = patch(
            &pool,
            c,
            &ck(&t),
            json!({"note":"晚點寄","visibility":"friends"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{close}: {b}");
        assert_eq!(b["claim"]["note"], "晚點寄");
        // 取消仍允許並回補數量
        assert_eq!(qty_claimed(&pool, it).await, 3);
        let (st, _, _) = call(&pool, "DELETE", &format!("/claims/{c}"), &ck(&t), None).await;
        assert_eq!(st, StatusCode::NO_CONTENT, "{close}");
        let (st, _, b) = patch(&pool, cp, &ck(&tp), json!({"status":"cancelled"})).await;
        assert_eq!(st, StatusCode::OK, "{close}: {b}");
        assert_eq!(qty_claimed(&pool, it).await, 0);
    }
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn legacy_guest_cannot_mark_purchased_after_close(pool: PgPool) {
    let o = user(&pool, "o").await;
    let wl = wishlist(
        &pool,
        o.id,
        ListOpts {
            status: Some("active"),
            ..Default::default()
        },
    )
    .await;
    let it = item(&pool, wl.id, "奶瓶", 2).await;
    let (g, c) = legacy_guest_claim(&pool, it, 1, None).await;
    sqlx::query(CLOSERS[0])
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, b) = patch(&pool, c, &gt(&g), json!({"status":"purchased"})).await;
    assert_eq!(st, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "WISHLIST_CLOSED");
    let (st, _, _) = call(&pool, "DELETE", &format!("/claims/{c}"), &gt(&g), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_still_delivers_and_cancels_on_closed_list(pool: PgPool) {
    for close in CLOSERS {
        let o = user(&pool, "o").await;
        let wl = wishlist(&pool, o.id, ListOpts::default()).await;
        let it = item(&pool, wl.id, "奶瓶", 3).await;
        let (_, c1) = user_claim(&pool, it, 1).await;
        let (_, c2) = user_claim(&pool, it, 1).await;
        sqlx::query(close).bind(wl.id).execute(&pool).await.unwrap();
        let (st, _, b) = patch(&pool, c1, &ck(&o.token), json!({"status":"delivered"})).await;
        assert_eq!(st, StatusCode::OK, "{close}: {b}");
        assert_eq!(b["claim"]["status"], "delivered");
        let (st, _, b) = patch(&pool, c2, &ck(&o.token), json!({"status":"cancelled"})).await;
        assert_eq!(st, StatusCode::OK, "{close}: {b}");
        assert_eq!(qty_claimed(&pool, it).await, 1);
    }
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn me_claims_reports_past_due_list_as_closed(pool: PgPool) {
    let o = user(&pool, "o").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 1).await;
    let (t, _) = user_claim(&pool, it, 1).await;
    sqlx::query(CLOSERS[2])
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, b) = call(&pool, "GET", "/guest/me", &ck(&t), None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["claims"][0]["wishlist"]["status"], "closed");
}
