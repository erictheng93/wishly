//! 擁有者推進與取消捐助。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_delivers_and_cancels_claims(pool: PgPool) {
    let TestUser { id: o, token: ot } = user(&pool, "u").await;
    let TestUser { id: _, token: st } = user(&pool, "u").await;
    let wl = wishlist(
        &pool,
        o,
        ListOpts {
            surprise: false,
            event_offset_days: Some(0),
            ..Default::default()
        },
    )
    .await;
    let wid = wl.id;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (_, c1) = user_claim(&pool, it, 1).await;
    let (_, c2) = user_claim(&pool, it, 1).await;
    let url = |c: Uuid| format!("/claims/{c}");
    // 路人 → 404（不洩漏存在）；擁有者改數量或 purchased → 403
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &url(c1),
            &ck(&st),
            Some(json!({"status":"delivered"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &url(c1),
            &ck(&ot),
            Some(json!({"status":"purchased"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&pool, "PATCH", &url(c1), &ck(&ot), Some(json!({"qty":2})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // 推進 delivered（qty_claimed 不變）
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &url(c1),
        &ck(&ot),
        Some(json!({"status":"delivered"})),
    )
    .await;
    assert_eq!(
        (
            s,
            v["claim"]["status"].as_str(),
            v["item"]["qty_claimed"].as_i64()
        ),
        (StatusCode::OK, Some("delivered"), Some(2)),
        "{v}"
    );
    // delivered 不可取消
    assert_eq!(
        call(&pool, "DELETE", &url(c1), &ck(&ot), None).await.0,
        StatusCode::CONFLICT
    );
    // 取消別人的捐助 → 回補、audit
    assert_eq!(
        call(&pool, "DELETE", &url(c2), &ck(&ot), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&pool, "DELETE", &url(c2), &ck(&ot), None).await.0,
        StatusCode::NO_CONTENT
    );
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(q, 1);
    let a: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type='user' AND actor_id=$1 AND action='claim.owner_update'").bind(o).fetch_one(&pool).await.unwrap();
    assert_eq!(a, 2);
    // 驚喜鎖定期間禁止
    let wl = wishlist(
        &pool,
        o,
        ListOpts {
            surprise: true,
            event_offset_days: Some(30),
            ..Default::default()
        },
    )
    .await;
    let it2 = item(&pool, wl.id, "奶瓶", 3).await;
    let (_, c3) = user_claim(&pool, it2, 1).await;
    assert_eq!(
        call(&pool, "DELETE", &url(c3), &ck(&ot), None).await.0,
        StatusCode::FORBIDDEN
    );
    let _ = wid;
    assert_all_reconciled(&pool).await;
}
