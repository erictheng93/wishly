//! 儀表板驚喜模式與下架資訊。
mod common;
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
async fn dashboard_locked_totals_and_moderation(pool: PgPool) {
    let (o, t) = user(&pool, false).await;
    let (wid, it) = list(&pool, o, true, 30).await;
    claim(&pool, it, None).await;
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&t),
        None,
    )
    .await;
    assert_eq!(v["totals"]["qty_claimed"], 1);
    assert!(v["items"][0]["qty_claimed"].is_null() && v["claims"].is_null());
    assert_eq!(v["moderation_status"], "ok");
    sqlx::query(
        "UPDATE wishlists SET moderation_status='hidden', moderation_reason='違規' WHERE id=$1",
    )
    .bind(wid)
    .execute(&pool)
    .await
    .unwrap();
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&t),
        None,
    )
    .await;
    assert_eq!(
        (
            v["moderation_status"].as_str(),
            v["moderation_reason"].as_str()
        ),
        (Some("hidden"), Some("違規"))
    );
    common::assert_claims_reconciled(&pool).await;
}
