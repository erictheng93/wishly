//! 唯讀旗標的寫入門檻。
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

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_mode_blocks_writes(pool: PgPool) {
    let (_, staff) = user(&pool, true).await;
    let (o, ot) = user(&pool, false).await;
    let (_, it) = list(&pool, o, false, 0).await;
    let (_, _, v) = call(&pool, "GET", "/admin/system-flags", &ck(&staff), None).await;
    assert_eq!(v["data"][0]["key"], "read_only");
    assert_eq!(v["data"][0]["value"], false);
    assert_eq!(
        call(&pool, "GET", "/admin/system-flags", &ck(&ot), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&staff),
            Some(json!({"value": true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, h, v) = call(
        &pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[("idempotency-key", Uuid::new_v4().to_string())],
        Some(json!({"qty":1,"display_name":"x"})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("READ_ONLY_MODE"))
    );
    assert!(h.contains_key("retry-after"));
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            &ck(&ot),
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(&pool, "GET", "/me/export", &ck(&ot), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&pool, "POST", "/auth/logout", &ck(&ot), None).await.0,
        StatusCode::NO_CONTENT
    );
    // 可關閉
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&staff),
            Some(json!({"value": false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(claim_status(&pool, it).await, StatusCode::CREATED);
    common::assert_claims_reconciled(&pool).await;
}

async fn claim_status(pool: &PgPool, it: Uuid) -> StatusCode {
    call(
        pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[("idempotency-key", Uuid::new_v4().to_string())],
        Some(json!({"qty":1,"display_name":"x"})),
    )
    .await
    .0
}
