//! F15 下架 / 恢復以目標狀態為準：重送不重寫 moderated_at、不重複 audit（流程驗證 W3）。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn moderation_resend_is_noop(pool: PgPool) {
    let TestUser { id: o, .. } = user(&pool, "o").await;
    let TestUser { token: st, .. } = staff(&pool, "s").await;
    let wl = wishlist(&pool, o, ListOpts::default()).await;
    let uri = format!("/admin/wishlists/{}/moderation", wl.id);
    let go = |b| {
        let (pool, h, uri) = (pool.clone(), ck(&st), uri.clone());
        async move { call(&pool, "PATCH", &uri, &h, Some(b)).await }
    };
    let audits = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_logs WHERE entity_id = $1 AND action = 'wishlist.moderate'",
        )
        .bind(wl.id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };

    let (s, _, first) = go(json!({"moderation_status":"hidden","reason":"疑似詐騙"})).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    let (s, _, again) = go(json!({"moderation_status":"hidden","reason":"疑似詐騙"})).await;
    assert_eq!(
        (s, &again["moderated_at"]),
        (StatusCode::OK, &first["moderated_at"]),
        "{again}"
    );
    assert_eq!(audits().await, 1);
    // 改原因是真的變更
    let (_, _, b) = go(json!({"moderation_status":"hidden","reason":"侵權"})).await;
    assert_eq!(b["moderation_reason"], "侵權");
    assert_eq!(audits().await, 2);
    // 恢復與重送恢復
    assert_eq!(
        go(json!({"moderation_status":"ok"})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        go(json!({"moderation_status":"ok"})).await.0,
        StatusCode::OK
    );
    assert_eq!(audits().await, 3);
}
