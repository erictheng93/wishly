//! 儀表板驚喜模式與下架資訊。
mod common;
use sqlx::PgPool;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn dashboard_locked_totals_and_moderation(pool: PgPool) {
    let TestUser { id: o, token: t } = user(&pool, "u").await;
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
    let wid = wl.id;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    user_claim(&pool, it, 1).await;
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
    assert_all_reconciled(&pool).await;
}
