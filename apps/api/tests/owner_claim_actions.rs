//! 擁有者推進與取消認領。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::api as call;
use common::ck;

async fn user(pool: &PgPool, staff: bool) -> (Uuid, String) {
    common::user_with_staff(pool, "u", staff).await
}

/// 清單 + 一個 qty_needed=3 的品項；event_offset_days>0 且 surprise → 鎖定
use common::guest_wishlist as list;

/// 訪客認領；回傳 (guest_token, claim_id)
use common::guest_claim as claim;

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_delivers_and_cancels_claims(pool: PgPool) {
    let (o, ot) = user(&pool, false).await;
    let (_, st) = user(&pool, false).await;
    let (wid, it) = list(&pool, o, false, 0).await;
    let (_, c1) = claim(&pool, it, None).await;
    let (_, c2) = claim(&pool, it, None).await;
    let url = |c: Uuid| format!("/claims/{c}");
    // 路人 / 擁有者改數量或 purchased → 403
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
        StatusCode::FORBIDDEN
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
    // 取消別人的認領 → 回補、audit
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
    let (_, it2) = list(&pool, o, true, 30).await;
    let (_, c3) = claim(&pool, it2, None).await;
    assert_eq!(
        call(&pool, "DELETE", &url(c3), &ck(&ot), None).await.0,
        StatusCode::FORBIDDEN
    );
    let _ = wid;
    common::assert_claims_reconciled(&pool).await;
}
